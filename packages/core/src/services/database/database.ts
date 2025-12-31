import { Surreal, SurrealTransaction } from 'surrealdb';

export abstract class AbstractDatabaseService {
  protected client: Surreal;

  constructor(client: Surreal) {
    this.client = client;
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
            const pending = this.client.query(query, vars);
            let result;
            if (pending && typeof pending.collect === 'function') {
              result = await pending.collect<T>();
            } else {
              result = await pending;
            }
            resolve(result as T);
          } catch (err) {
            reject(err);
          }
        })
        .catch(() => {
          // Ignore queue errors to keep the chain alive; the specific promise was rejected above.
        });
    });
  }

  async close(): Promise<void> {
    await this.client.close();
  }
}
