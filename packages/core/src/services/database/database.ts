import { Surreal, SurrealTransaction } from 'surrealdb';
import { createLogger, Logger } from '../logger/index.js';

export abstract class AbstractDatabaseService {
  protected client: Surreal;
  protected logger: Logger;

  constructor(client: Surreal, logger: Logger) {
    this.client = client;
    this.logger = logger.child({ service: 'Database' });
  }

  abstract connect(): Promise<void>;

  getClient(): Surreal {
    return this.client;
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
          try {
            this.logger.debug({ query, vars }, 'Executing query');
            const pending = this.client.query(query, vars);
            // In SurrealDB 2.0, .query() collects results by default.
            // We cast to T directly as proper typing depends on the caller knowing the return structure.
            const result = (await pending) as unknown as T;
            resolve(result);
            this.logger.trace({ query, result }, 'Query executed successfully');
          } catch (err) {
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
