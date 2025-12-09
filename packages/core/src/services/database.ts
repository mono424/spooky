import { Surreal } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { SpookyConfig } from "../types.js";

export class DatabaseService {
  private local: Surreal;
  private remote: Surreal;
  private config: SpookyConfig;

  constructor(config: SpookyConfig) {
    this.config = config;
    this.local = new Surreal({
      engines: createWasmEngines(),
    });
    this.remote = new Surreal();
  }

  async init() {
    // Initialize local database
    await this.local.connect("indxdb://spooky");
    await this.local.use({
      namespace: this.config.database.namespace,
      database: this.config.database.database,
    });

    // Initialize remote database if endpoint is provided
    if (this.config.database.endpoint) {
      await this.remote.connect(this.config.database.endpoint);
      await this.remote.use({
        namespace: this.config.database.namespace,
        database: this.config.database.database,
      });
      
      if (this.config.database.token) {
        await this.remote.authenticate(this.config.database.token);
      }
    }
  }

  async queryLocal<T>(sql: string, vars?: Record<string, unknown>): Promise<T> {
    const result = await this.local.query(sql, vars);
    // @ts-ignore
    if (typeof result.collect === 'function') {
      // @ts-ignore
      const collected = await result.collect();
      return collected[0] as T;
    }
    return result as unknown as T;
  }

  async queryRemote<T>(sql: string, vars?: Record<string, unknown>): Promise<T> {
    const result = await this.remote.query(sql, vars);
    // @ts-ignore
    if (typeof result.collect === 'function') {
      // @ts-ignore
      const collected = await result.collect();
      return collected[0] as T;
    }
    return result as unknown as T;
  }

  async subscribeLive(uuid: string, callback: (action: string, result: Record<string, unknown>) => void) {
    // @ts-ignore
    if (typeof this.remote.liveQuery === 'function') {
      // @ts-ignore
      const iterator = this.remote.liveQuery(uuid);
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

  getLocal(): Surreal {
    return this.local;
  }

  getRemote(): Surreal {
    return this.remote;
  }

  async close() {
    await this.local.close();
    await this.remote.close();
  }
}
