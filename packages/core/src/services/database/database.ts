import type { Surreal, SurrealTransaction } from 'surrealdb';
import type { Logger } from '../logger/index';
import type {
  DatabaseEventSystem,
  DatabaseEventTypes} from './events/index';
import type { SealedQuery } from '../../utils/surql';
import { withTimeout } from '../../utils/index';

export abstract class AbstractDatabaseService {
  protected client: Surreal;
  protected logger: Logger;
  protected events: DatabaseEventSystem;
  /**
   * Per-query deadline in ms; `0` disables. Only the remote service sets this
   * (see `RemoteDatabaseService`) — a local query can be legitimately slow and
   * has its own retry ladders, and there is no half-open-socket failure mode
   * for an in-process engine.
   */
  protected queryTimeoutMs = 0;
  protected abstract eventType:
    | typeof DatabaseEventTypes.LocalQuery
    | typeof DatabaseEventTypes.RemoteQuery;

  constructor(client: Surreal, logger: Logger, events: DatabaseEventSystem) {
    this.client = client;
    this.logger = logger.child({ service: 'Database' });
    this.events = events;
  }

  abstract connect(): Promise<void>;

  getClient(): Surreal {
    return this.client;
  }

  getEvents(): DatabaseEventSystem {
    return this.events;
  }

  tx(): Promise<SurrealTransaction> {
    return this.client.beginTransaction();
  }

  private queryQueue: Promise<void> = Promise.resolve();

  /**
   * Execute a query with serialized execution to prevent WASM transaction issues.
   *
   * Serialization means every query waits on the previous one, so a call that
   * never settles blocks the whole chain forever. {@link queryTimeoutMs} bounds
   * each link: on expiry this promise rejects and the chain moves on, even
   * though the underlying RPC is still parked in the SDK's pending map.
   */
  async query<T extends unknown[]>(query: string, vars?: Record<string, unknown>): Promise<T> {
    return new Promise((resolve, reject) => {
      this.queryQueue = this.queryQueue
        // oxlint-disable-next-line promise/always-return
        .then(async () => {
          const startTime = performance.now();
          try {
            this.logger.debug(
              { query, vars, Category: 'sp00ky-client::Database::query' },
              'Executing query'
            );
            const pending = this.client.query(query, vars);
            // In SurrealDB 2.0, .query() collects results by default.
            // We cast to T directly as proper typing depends on the caller knowing the return structure.
            // "timed out" in the message is load-bearing: `classifySyncError`
            // keys off it to classify this as `network` so the sync queues
            // retry rather than rolling the mutation back.
            const result = (await withTimeout(
              pending as unknown as Promise<T>,
              this.queryTimeoutMs,
              `Remote query timed out after ${this.queryTimeoutMs}ms`
            )) as T;
            const duration = performance.now() - startTime;

            // Emit query event
            this.events.emit(this.eventType, {
              query,
              vars,
              duration,
              success: true,
              timestamp: Date.now(),
            });

            resolve(result);
            this.logger.trace(
              { query, result, Category: 'sp00ky-client::Database::query' },
              'Query executed successfully'
            );
          } catch (err) {
            const duration = performance.now() - startTime;

            // Emit query event with error
            this.events.emit(this.eventType, {
              query,
              vars,
              duration,
              success: false,
              error: err instanceof Error ? err.message : String(err),
              timestamp: Date.now(),
            });

            this.logger.error(
              { query, vars, err, Category: 'sp00ky-client::Database::query' },
              'Query execution failed'
            );
            // oxlint-disable-next-line no-multiple-resolved -- resolve/reject are in try/catch, mutually exclusive
            reject(err);
          }
        })
        .catch(() => {
          // Ignore queue errors to keep the chain alive; the specific promise was rejected above.
        });
    });
  }

  async execute<T>(query: SealedQuery<T>, vars?: Record<string, unknown>): Promise<T> {
    const raw = await this.query<unknown[]>(query.sql, vars);
    return query.extract(raw);
  }

  async close(): Promise<void> {
    this.logger.info({ Category: 'sp00ky-client::Database::close' }, 'Closing database connection');
    await this.client.close();
  }
}
