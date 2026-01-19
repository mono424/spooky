import { Surreal, SurrealTransaction } from 'surrealdb';
import { createLogger, Logger } from '../logger/index.js';
import {
  DatabaseEventSystem,
  DatabaseEventTypes,
  DatabaseQueryEventPayload,
} from './events/index.js';

export abstract class AbstractDatabaseService {
  protected client: Surreal;
  protected logger: Logger;
  protected events: DatabaseEventSystem;
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
   */
  async query<T extends unknown[]>(query: string, vars?: Record<string, unknown>): Promise<T> {
    return new Promise((resolve, reject) => {
      this.queryQueue = this.queryQueue
        .then(async () => {
          const startTime = performance.now();
          try {
            this.logger.debug({ query, vars }, 'Executing query');
            const pending = this.client.query(query, vars);
            // In SurrealDB 2.0, .query() collects results by default.
            // We cast to T directly as proper typing depends on the caller knowing the return structure.
            const result = (await pending) as unknown as T;
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
            this.logger.trace({ query, result }, 'Query executed successfully');
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

            this.logger.error({ query, vars, err }, 'Query execution failed');
            reject(err);
          }
        })
        .catch(() => {
          // Ignore queue errors to keep the chain alive; the specific promise was rejected above.
        });
    });
  }

  async close(): Promise<void> {
    this.logger.info('Closing database connection');
    await this.client.close();
  }
}
