import { Surreal, SurrealTransaction } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { SpookyConfig } from "../../types.js";
import { AbstractDatabaseService } from "./database.js";

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database']) {
    super(new Surreal({
      engines: createWasmEngines(),
    }));
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  tx(): Promise<SurrealTransaction> {
    return this.client.beginTransaction();
  }

  async connect(): Promise<void> {
    const { namespace, database } = this.getConfig();
    await this.client.connect("indxdb://spooky");
    await this.client.use({
      namespace,
      database,
    });
  }
}
