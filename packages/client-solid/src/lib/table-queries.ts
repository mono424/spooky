import { RecordId, Surreal, Table, Values } from "surrealdb";
import { GenericModel, GenericSchema, Model } from "./models";
import { QueryResponse, SyncedDb } from "..";

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
    data: Values<Model<T>> | Values<Model<T>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createLocal<T>(this.table, data);
  }

  createRemote<T extends GenericModel = Schema[K]>(
    data: Values<Model<T>> | Values<Model<T>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createRemote<T>(this.table, data);
  }

  updateLocal<T extends GenericModel = Schema[K]>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateLocal<T>(recordId, data);
  }

  updateRemote<T extends GenericModel = Schema[K]>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateRemote<T>(recordId, data);
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
export class QueryNamespace<Schema extends GenericSchema> {
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
