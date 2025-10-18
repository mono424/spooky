import { RecordResult, Surreal, Table, Values } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { SchemaProvisioner } from "./schema/provisioner";
import { createSurrealDBWasm } from "./cache";
import type { SyncedDbConfig, DbConnection } from "./types";
export type { RecordResult } from "surrealdb";

type GenericModel = { id: string };
type GenericSchema = Record<string, GenericModel>;

export type Model<T extends GenericModel> = RecordResult<Omit<T, "id">>;

/**
 * Generic query response carrying Surreal query handle plus typed collect()
 */
export type QueryResponse<T extends GenericModel> = Omit<
  ReturnType<Surreal["query"]>,
  "collect"
> & {
  collect: () => Promise<[Model<T>[]]>;
};

/**
 * Table query interface for a specific table. The response type is inferred from Schema[K].
 */
class TableQuery<
  Schema extends GenericSchema,
  K extends keyof Schema & string
> {
  private table: Table;

  constructor(private db: SyncedDb<Schema>, public readonly tableName: K) {
    this.table = new Table(this.tableName);
  }

  queryLocal(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<Schema[K]> {
    return this.db.queryLocal<Schema[K]>(sql, vars);
  }

  queryRemote(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<Schema[K]> {
    return this.db.queryRemote<Schema[K]>(sql, vars);
  }

  createLocal<T extends GenericModel = Schema[K]>(
    data?: Values<Model<T>>
  ): ReturnType<Surreal["create"]> {
    return this.db.createLocal<T>(this.table, data);
  }

  createRemote<T extends GenericModel = Schema[K]>(
    data?: Values<Model<T>>
  ): ReturnType<Surreal["create"]> {
    return this.db.createRemote<T>(this.table, data);
  }

  updateLocal<T extends GenericModel = Schema[K]>(
    table: Table
  ): ReturnType<Surreal["update"]> {
    return this.db.updateLocal<T>(table);
  }

  updateRemote<T extends GenericModel = Schema[K]>(
    table: Table
  ): ReturnType<Surreal["update"]> {
    return this.db.updateRemote<T>(table);
  }

  deleteLocal<T extends GenericModel = Schema[K]>(
    table: Table
  ): ReturnType<Surreal["delete"]> {
    return this.db.deleteLocal<T>(table);
  }

  deleteRemote<T extends GenericModel = Schema[K]>(
    table: Table
  ): ReturnType<Surreal["delete"]> {
    return this.db.deleteRemote<T>(table);
  }
}

/**
 * Query namespace that provides table-scoped query access
 */
class QueryNamespace<Schema extends GenericSchema> {
  private tableCache = new Map<
    keyof Schema & string,
    TableQuery<Schema, any>
  >();

  constructor(private db: SyncedDb<Schema>) {
    // Create a proxy to handle dynamic table access
    return new Proxy(this, {
      get(target, prop: keyof Schema | string | symbol) {
        if (
          typeof prop === "string" &&
          prop !== "tableCache" &&
          prop !== "db"
        ) {
          const key = prop as keyof Schema & string;
          if (!target.tableCache.has(key)) {
            target.tableCache.set(
              key,
              new TableQuery<Schema, typeof key>(target.db, key)
            );
          }
          return target.tableCache.get(key);
        }
        return Reflect.get(target, prop);
      },
    }) as QueryNamespace<Schema>;
  }
}

/**
 * Type helper for table queries
 */
export type TableQueries<Schema extends GenericSchema> = {
  [K in keyof Schema & string]: TableQuery<Schema, K>;
};

export class SyncedDb<Schema extends GenericSchema> {
  private config: SyncedDbConfig;
  private connections: DbConnection | null = null;
  public readonly query: QueryNamespace<Schema> & TableQueries<Schema>;

  constructor(config: SyncedDbConfig) {
    this.config = config;
    this.query = new QueryNamespace<Schema>(this) as QueryNamespace<Schema> &
      TableQueries<Schema>;
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
    } = this.config;

    // Internal WASM database
    const internal = await createSurrealDBWasm(
      internalDbName,
      storageStrategy,
      namespace,
      database
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
      remote = new Surreal({ engines: createWasmEngines() });
      await remote.connect(remoteUrl);

      if (namespace || database) {
        await remote.use({
          namespace: namespace || "main",
          database: database || "main",
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
      namespace || "main",
      database || "main"
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
      collect: () => {
        return oCollect<[Model<T>[]]>();
      },
    });
  }

  _create<T extends GenericModel>(
    db: Surreal,
    table: Table,
    data?: Values<Model<T>>
  ): ReturnType<Surreal["create"]> {
    return db.create<Model<T>>(table, data);
  }

  _update<T extends Record<string, unknown> = Record<string, unknown>>(
    db: Surreal,
    table: Table
  ): ReturnType<Surreal["update"]> {
    return db.update<T>(table);
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
    data?: Values<Model<T>>
  ): ReturnType<Surreal["create"]> {
    const db = this.getLocal();
    return this._create<T>(db, thing, data);
  }

  createRemote<T extends GenericModel>(
    table: Table,
    data?: Values<Model<T>>
  ): ReturnType<Surreal["create"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._create<T>(db, table, data);
  }

  updateLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table
  ): ReturnType<Surreal["update"]> {
    const db = this.getLocal();
    return this._update<T>(db, table);
  }

  updateRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table
  ): ReturnType<Surreal["update"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._update<T>(db, table);
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
