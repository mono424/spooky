import {
  Frame,
  LiveSubscription,
  RecordId,
  Surreal,
  Table,
  Values,
} from "surrealdb";
import { GenericModel, GenericSchema, ModelPayload } from "./models";
import { QueryResponse, SyncedDb } from "..";

/**
 * Table query interface for a specific table. The response type is inferred from Schema[K].
 */
class TableQuery<Schema extends GenericSchema, Model extends GenericModel> {
  private table: Table;

  constructor(private db: SyncedDb<Schema>, public readonly tableName: string) {
    this.table = new Table(this.tableName);
  }

  liveQuery({
    select,
    where,
    orderBy,
  }: {
    select?: ((keyof Model & string) | "*")[];
    where?: Partial<Model>;
    orderBy?: Partial<Record<keyof Model, "asc" | "desc">>;
  }): AsyncIterable<Frame<Model, false>> {
    const selectClause = (select ?? ["*"]).map((key) => `${key}`).join(", ");

    const whereClause = Object.keys(where ?? {})
      .map((key) => `${key} = $${key}`)
      .join(" AND ");

    const orderClause = Object.entries(orderBy ?? {})
      .map(([key, val]) => `${key} ${val}`)
      .join(", ");

    const query = `LIVE SELECT ${selectClause}
    FROM ${this.table.name}
    ${whereClause ? `WHERE ${whereClause}` : ""}
    ${orderClause ? `ORDER BY ${orderClause}` : ""}`;

    return this.db.queryLocal<Model>(query, where).stream();
  }

  queryLocal(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<Model> {
    return this.db.queryLocal<Model>(sql, vars);
  }

  queryRemote(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<Model> {
    return this.db.queryRemote<Model>(sql, vars);
  }

  createLocal(
    data: Values<ModelPayload<Model>> | Values<ModelPayload<Model>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createLocal<Model>(this.table, data);
  }

  createRemote(
    data: Values<ModelPayload<Model>> | Values<ModelPayload<Model>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createRemote<Model>(this.table, data);
  }

  updateLocal(
    recordId: RecordId,
    data: Partial<Model>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateLocal<Model>(recordId, data);
  }

  updateRemote(
    recordId: RecordId,
    data: Partial<Model>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateRemote<Model>(recordId, data);
  }

  deleteLocal(table: Table): ReturnType<Surreal["delete"]> {
    return this.db.deleteLocal<Model>(table);
  }

  deleteRemote(table: Table): ReturnType<Surreal["delete"]> {
    return this.db.deleteRemote<Model>(table);
  }
}

/**
 * Query namespace that provides table-scoped query access
 */
export class QueryNamespace<Schema extends GenericSchema> {
  private tableCache = new Map<
    keyof Schema & string,
    TableQuery<Schema, Schema[keyof Schema & string]>
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
          console.log(key, typeof key);
          if (!target.tableCache.has(key)) {
            target.tableCache.set(
              key,
              new TableQuery<Schema, Schema[keyof Schema & string]>(
                target.db,
                key
              )
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
  [K in keyof Schema & string]: TableQuery<Schema, Schema[K]>;
};
