import { Surreal } from "surrealdb";
import { SpookyConfig } from "../../types.js";
import { AbstractDatabaseService } from "./database.js";

export class RemoteDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig;

  constructor(config: SpookyConfig) {
    super(new Surreal());
    this.config = config;
  }

  async connect(): Promise<void> {
    if (this.config.database.endpoint) {
      await this.client.connect(this.config.database.endpoint);
      await this.client.use({
        namespace: this.config.database.namespace,
        database: this.config.database.database,
      });
      
      if (this.config.database.token) {
        await this.client.authenticate(this.config.database.token);
      }
    }
  }

  async subscribeLive(uuid: string, callback: (action: string, result: Record<string, unknown>) => void) {
    // @ts-ignore
    if (typeof this.client.liveQuery === 'function') {
      // @ts-ignore
      const iterator = this.client.liveQuery(uuid);
      (async () => {
        try {
          for await (const msg of iterator) {
            callback(msg.action, msg.result as Record<string, unknown>);
          }
        } catch (e) {
          console.error("Live query loop error", e);
        }
      })();
    }
  }
}
