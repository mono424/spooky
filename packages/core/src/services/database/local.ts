import { Surreal } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { SpookyConfig } from "../../types.js";
import { AbstractDatabaseService } from "./database.js";

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig;

  constructor(config: SpookyConfig) {
    super(new Surreal({
      engines: createWasmEngines(),
    }));
    this.config = config;
  }

  async connect(): Promise<void> {
    await this.client.connect("indxdb://spooky");
    await this.client.use({
      namespace: this.config.database.namespace,
      database: this.config.database.database,
    });
  }
}
