import {
  RecordId,
  Surreal,
  Table,
  Values,
  createRemoteEngines,
} from "surrealdb";
import { SchemaProvisioner } from "./schema/provisioner";
import { createSurrealDBWasm } from "./cache";
import type { SyncedDbConfig, DbConnection } from "./types";
import { GenericModel, GenericSchema, Model } from "./lib/models";
import { QueryNamespace, TableQueries } from "./lib/table-queries";
export type { RecordResult } from "surrealdb";
export type { Model } from "./lib/models";

export { RecordId } from "surrealdb";

export type QueryResponse<T extends GenericModel> = Omit<
  ReturnType<Surreal["query"]>,
  "collect"
> & {
  collect: () => Promise<[Model<T>[]]>;
};

export class SyncedDb<Schema extends GenericSchema> {
  private config: SyncedDbConfig;
  private connections: DbConnection | null = null;
  public readonly query: QueryNamespace<Schema> & TableQueries<Schema>;

  constructor(config: SyncedDbConfig) {
    this.config = config;
    this.query = new QueryNamespace<Schema>(this) as QueryNamespace<Schema> &
      TableQueries<Schema>;
    window.db = this;
  }

  logDatabase(db: Surreal) {
    console.log(`Database[cache] version: ${db.version}`);
    console.log(`Database[cache] config: ${db.status}`);
  }

  /**
   * Initialize local WASM DB and optional remote client, then provision local schema
   */
  async init(): Promise<void> {
    const {
      localDbName,
      internalDbName,
      storageStrategy,
      namespace,
      database,
      remoteUrl,
      token,
      schema,
    } = this.config;

    // Internal WASM database
    const internal = await createSurrealDBWasm(
      internalDbName,
      storageStrategy,
      "internal",
      "main"
    );

    // Local WASM database
    const local = await createSurrealDBWasm(
      localDbName,
      storageStrategy,
      namespace,
      database
    );

    // Optional remote HTTP client
    let remote: Surreal | undefined;
    if (remoteUrl) {
      remote = new Surreal({ engines: createRemoteEngines() });
      await remote.connect(remoteUrl);

      if (namespace || database) {
        await remote.use({
          namespace: namespace,
          database: database,
        });
      }
      if (token) {
        await remote.authenticate(token);
      }
    }

    this.connections = { local, internal, remote };

    // Provision local schema from src/db/schema.surql
    const provisioner = new SchemaProvisioner(
      internal,
      local,
      namespace,
      database,
      schema
    );
    await provisioner.provision();
  }

  getLocal(): Surreal {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");
    return this.connections.local;
  }

  getRemote(): Surreal | undefined {
    return this.connections?.remote;
  }

  _query<T extends GenericModel>(
    db: Surreal,
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const res = db.query(sql, vars as any);
    const oCollect = res.collect.bind(res);
    return Object.assign(res, {
      collect: () => oCollect<[Model<T>[]]>(),
    });
  }

  _create<T extends GenericModel>(
    db: Surreal,
    table: Table,
    data: Values<Model<T>> | Values<Model<T>>[]
  ): ReturnType<Surreal["insert"]> {
    console.log("createLocal", table, data);
    return db.insert<Model<T>>(table, data);
  }

  _update<T extends Record<string, unknown> = Record<string, unknown>>(
    db: Surreal,
    recordId: RecordId,
    data: Partial<T>
  ): Promise<any> {
    return db.update<T>(recordId).merge(data);
  }

  _delete<T extends Record<string, unknown> = Record<string, unknown>>(
    db: Surreal,
    table: Table
  ): ReturnType<Surreal["delete"]> {
    return db.delete<T>(table);
  }

  queryLocal<T extends GenericModel>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getLocal();
    return this._query<T>(db, sql, vars);
  }

  queryRemote<T extends GenericModel>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._query<T>(db, sql, vars);
  }

  createLocal<T extends GenericModel>(
    thing: Table,
    data: Values<Model<T>> | Values<Model<T>>[]
  ): ReturnType<Surreal["create"]> {
    const db = this.getLocal();
    return this._create<T>(db, thing, data);
  }

  createRemote<T extends GenericModel>(
    table: Table,
    data: Values<Model<T>> | Values<Model<T>>[]
  ): ReturnType<Surreal["create"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._create<T>(db, table, data);
  }

  updateLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    const db = this.getLocal();
    return this._update<T>(db, recordId, data);
  }

  updateRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._update<T>(db, recordId, data);
  }

  deleteLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table
  ): ReturnType<Surreal["delete"]> {
    const db = this.getLocal();
    return this._delete<T>(db, table);
  }

  deleteRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table
  ): ReturnType<Surreal["delete"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._delete<T>(db, table);
  }
}

export * from "./types";
