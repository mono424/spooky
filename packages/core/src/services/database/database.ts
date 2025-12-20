import { Surreal } from "surrealdb";

export abstract class AbstractDatabaseService {
  protected client: Surreal;

  constructor(client: Surreal) {
    this.client = client;
  }

  abstract connect(): Promise<void>;

  getClient(): Surreal {
    return this.client;
  }

  async close(): Promise<void> {
    await this.client.close();
  }
}
