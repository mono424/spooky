import { Surreal } from "surrealdb";

export abstract class AbstractDatabaseService {
  protected client: Surreal;

  constructor(client: Surreal) {
    this.client = client;
  }

  abstract connect(): Promise<void>;

  async query<T>(sql: string, vars?: Record<string, unknown>): Promise<T> {
    const result = await this.client.query(sql, vars);
    // @ts-ignore
    if (typeof result.collect === 'function') {
      // @ts-ignore
      const collected = await result.collect();
      return collected[0] as T;
    }
    return result as unknown as T;
  }

  getClient(): Surreal {
    return this.client;
  }

  async close(): Promise<void> {
    await this.client.close();
  }
}
