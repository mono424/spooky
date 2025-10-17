import { Surreal } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { SchemaProvisioner } from "./schema/provisioner";
import { createSurrealDBWasm } from "./cache";
import type { SyncedDbConfig, DbConnection } from "./types";

/**
 * Generic query response carrying Surreal query handle plus typed collect()
 */
export type QueryResponse<T> = Omit<ReturnType<Surreal["query"]>, "collect"> & {
  collect: () => Promise<[T[]]>;
};

/**
 * Table query interface for a specific table. The response type is inferred from Schema[K].
 */
class TableQuery<Schema, K extends keyof Schema & string> {
  constructor(private db: SyncedDb<Schema>, public readonly tableName: K) {}

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
}

/**
 * Query namespace that provides table-scoped query access
 */
class QueryNamespace<Schema = any> {
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
export type TableQueries<Schema> = {
  [K in keyof Schema & string]: TableQuery<Schema, K>;
};

export class SyncedDb<Schema = any> {
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

  _query<T = unknown>(
    db: Surreal,
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const res = db.query(sql, vars as any);
    const oCollect = res.collect.bind(res);
    return Object.assign(res, {
      collect: () => {
        return oCollect<[T[]]>();
      },
    });
  }

  queryLocal<T = unknown>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getLocal();
    return this._query<T>(db, sql, vars);
  }

  queryRemote<T = unknown>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._query<T>(db, sql, vars);
  }
}

export * from "./types";
