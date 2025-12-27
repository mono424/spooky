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

  async close(): Promise<void> {
    await this.client.close();
  }

  async query<T>(query: string, vars?: Record<string, unknown>): Promise<T> {
    const pending = this.client.query(query, vars) as any;
    if (pending && typeof pending.collect === 'function') {
      return (await pending.collect()) as T;
    }
    return (await pending) as T;
  }
}
