import {
  Frame,
  LiveSubscription,
  RecordId,
  Surreal,
  Table,
  Uuid,
  Values,
} from "surrealdb";
import { GenericModel, GenericSchema, ModelPayload } from "./models";
import { QueryResponse, SyncedDb } from "..";

export interface LiveQueryOptions<Model extends GenericModel> {
  select?: ((keyof Model & string) | "*")[];
  where?: Partial<Model> | { id: RecordId };
  orderBy?: Partial<Record<keyof Model, "asc" | "desc">>;
}

export interface QueryInfo {
  query: string;
  vars?: Record<string, unknown>;
}

export class LiveQueryList<
  Schema extends GenericSchema,
  Model extends GenericModel
> {
  private state: Model[];
  private liveQ: LiveSubscription | undefined;

  constructor(
    private hydrationQuery: QueryInfo,
    private liveQuery: QueryInfo,
    private db: SyncedDb<Schema>,
    private callback: (items: Model[]) => void
  ) {
    this.state = [];
    this.hydrationQuery = hydrationQuery;
    this.liveQuery = liveQuery;
    this.db = db;
    this.callback = callback;
  }

  private async hydrate(): Promise<void> {
    const [models] = await this.db
      .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
      .collect();
    this.state = models as Model[];
    console.log("[LiveQueryList] Hydrated", this.state);
    this.callback(this.state);
  }

  private async initLive(): Promise<void> {
    const [queryId] = (await this.db
      .queryLocal(this.liveQuery.query, this.liveQuery.vars)
      .collect()) as unknown as [Uuid];
    this.liveQ = await this.db.getLocal().liveOf(queryId);
    this.liveQ.subscribe((event) => {
      event.value = {
        ...event.value,
        id: (event?.value?.id as RecordId)?.id,
      };
      if (event.action === "DELETE") {
        this.state = this.state.filter((item) => item.id !== event.value.id);
      } else if (event.action === "CREATE") {
        this.state.push(event.value as Model);
      } else if (event.action === "UPDATE") {
        this.state = this.state.map((item) =>
          item.id === event.value.id ? (event.value as Model) : item
        );
      }
      console.log("[LiveQueryList] Live", this.state);
      this.callback(this.state);
    });
  }

  public async init(): Promise<void> {
    await this.hydrate();
    await this.initLive();
  }

  public kill(): void {
    this.liveQ?.kill();
  }
}

/**
 * Table query interface for a specific table. The response type is inferred from Schema[K].
 */
export class TableQuery<
  Schema extends GenericSchema,
  Model extends GenericModel
> {
  private table: Table;

  constructor(private db: SyncedDb<Schema>, public readonly tableName: string) {
    this.table = new Table(this.tableName);
  }

  private buildQuery(
    method: "LIVE SELECT" | "SELECT",
    props: LiveQueryOptions<Model>
  ): QueryInfo {
    const selectClause = (props.select ?? ["*"])
      .map((key) => `${key}`)
      .join(", ");
    const whereClause = Object.keys(props.where ?? {})
      .map((key) => `${key} = $${key}`)
      .join(" AND ");
    const orderClause = Object.entries(props.orderBy ?? {})
      .map(([key, val]) => `${key} ${val}`)
      .join(", ");

    return {
      query: `${method} ${selectClause}
      FROM ${this.tableName}
      ${whereClause ? `WHERE ${whereClause}` : ""}
      ${orderClause ? `ORDER BY ${orderClause}` : ""}`,
      vars: props.where,
    };
  }

  async liveQuery(
    options: LiveQueryOptions<Model>,
    callback: (queryId: Model[]) => void
  ): Promise<LiveQueryList<Schema, Model>> {
    const liveQuery = new LiveQueryList<Schema, Model>(
      this.buildQuery("SELECT", options),
      this.buildQuery("LIVE SELECT", options),
      this.db,
      callback
    );

    await liveQuery.init();
    return liveQuery;
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
