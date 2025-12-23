import { Surreal, SurrealTransaction } from "surrealdb";
import { SpookyConfig } from "../../types.js";
import { AbstractDatabaseService } from "./database.js";

export class RemoteDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database']) {
    super(new Surreal());
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }
  
  async connect(): Promise<void> {
    const {endpoint, token, namespace, database} = this.getConfig();
    if (endpoint) {
      await this.client.connect(endpoint);
      await this.client.use({
        namespace,
        database,
      });
      
      if (token) {
        await this.client.authenticate(token);
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
