import {
  Frame,
  RecordId,
  Surreal,
  Table,
  Values,
} from "surrealdb";
import { proxy } from "valtio";
import { GenericModel, GenericSchema, ModelPayload } from "./models";
import { QueryResponse, SyncedDb } from "..";

export interface LiveQueryOptions<Model extends GenericModel> {
  select?: ((keyof Model & string) | "*")[];
  where?: Partial<Model> | { id: RecordId };
  orderBy?: Partial<Record<keyof Model, "asc" | "desc">>;
  limit?: number;
  offset?: number;
}

export interface QueryInfo {
  query: string;
  vars?: Record<string, unknown>;
}

/**
 * Reactive query result with live updates
 */
export class ReactiveQueryResult<Model extends GenericModel> {
  private state: Model[];
  private liveQuery: LiveQueryList<any, Model> | null = null;

  constructor() {
    this.state = proxy([]) as Model[];
  }

  get data(): Model[] {
    return this.state;
  }

  _setLiveQuery(liveQuery: LiveQueryList<any, Model>): void {
    this.liveQuery = liveQuery;
  }

  _updateState(newState: Model[]): void {
    // Clear and replace array contents to maintain proxy reference
    this.state.length = 0;
    this.state.push(...newState);
  }

  /**
   * Stop listening to live updates
   */
  kill(): void {
    this.liveQuery?.kill();
  }
}

export class LiveQueryList<
  Schema extends GenericSchema,
  Model extends GenericModel
> {
  private state: Model[];
  private unsubscribe: (() => void) | undefined;

  constructor(
    private hydrationQuery: QueryInfo,
    private tableName: string,
    private db: SyncedDb<Schema>,
    private callback: (items: Model[]) => void
  ) {
    this.state = [];
    this.hydrationQuery = hydrationQuery;
    this.tableName = tableName;
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

  private async initRemoteLive(): Promise<void> {
    const syncer = this.db.getSyncer();
    if (!syncer || !syncer.isActive()) {
      console.warn(
        "[LiveQueryList] No syncer available, live updates will not work"
      );
      return;
    }

    console.log("[LiveQueryList] Setting up remote live query", {
      query: this.hydrationQuery.query,
      table: this.tableName,
    });

    // Subscribe to remote live query via syncer
    this.unsubscribe = await syncer.subscribeLiveQuery(
      this.hydrationQuery.query,
      this.hydrationQuery.vars,
      [this.tableName],
      async () => {
        // Re-hydrate from local cache when remote changes
        await this.hydrate();
      }
    );
  }

  public async init(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 10));
    await this.hydrate();
    await this.initRemoteLive();
  }

  public kill(): void {
    if (this.unsubscribe) {
      this.unsubscribe();
      this.unsubscribe = undefined;
    }
  }
}

/**
 * Fluent query builder for constructing queries with chainable methods
 */
export class QueryBuilder<
  Schema extends GenericSchema,
  Model extends GenericModel,
  TableName extends keyof Schema & string = keyof Schema & string
> {
  private options: LiveQueryOptions<Model> = {};
  private relatedTables: string[] = [];

  constructor(
    private tableQuery: TableQuery<Schema, Model>,
    private currentTableName: TableName,
    where?: Partial<Model>
  ) {
    if (where) {
      this.options.where = where;
    }
  }

  /**
   * Add additional where conditions
   */
  where(conditions: Partial<Model>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  /**
   * Specify fields to select
   */
  select(...fields: ((keyof Model & string) | "*")[]): this {
    if (this.options.select) {
      throw new Error("Select can only be called once per query");
    }
    this.options.select = fields;
    return this;
  }

  /**
   * Add ordering to the query
   */
  orderBy(
    field: keyof Model & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<Record<keyof Model, "asc" | "desc">>;
    return this;
  }

  /**
   * Limit the number of results
   */
  limit(count: number): this {
    this.options.limit = count;
    return this;
  }

  /**
   * Set the offset for results
   */
  offset(count: number): this {
    this.options.offset = count;
    return this;
  }

  /**
   * Include related records from specified table(s)
   * @example
   * // For a thread table that has relationship to user
   * tableQuery.find().related("user").query()
   *
   * @param relatedTable - The name of the related table to include
   */
  related<RelatedTable extends string>(
    relatedTable: RelatedTable
  ): this {
    if (!this.relatedTables.includes(relatedTable)) {
      this.relatedTables.push(relatedTable);
    }
    return this;
  }

  /**
   * Execute the query and return a reactive result that updates automatically
   * @example
   * const result = await tableQuery.find({ status: 'active' }).query();
   * // result.data is a reactive array that updates in real-time
   * console.log(result.data); // access the reactive array
   *
   * // Clean up when done
   * result.kill();
   */
  async query(): Promise<ReactiveQueryResult<Model>> {
    const result = new ReactiveQueryResult<Model>();

    // Set up live query with callback that updates the reactive state
    const liveQuery = await this.tableQuery.liveQuery(this.options, (items) => {
      result._updateState(items);
    });

    result._setLiveQuery(liveQuery);
    return result;
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

    let query = `${method} ${selectClause} FROM ${this.tableName}`;
    if (whereClause) query += ` WHERE ${whereClause}`;
    if (orderClause) query += ` ORDER BY ${orderClause}`;
    if (props.limit !== undefined) query += ` LIMIT ${props.limit}`;
    if (props.offset !== undefined) query += ` START ${props.offset}`;
    query += ";";

    return {
      query,
      vars: props.where,
    };
  }

  /**
   * Start a fluent query with optional where conditions
   * @example
   * tableQuery.find({ status: 'active' }).orderBy('createdAt', 'desc').query()
   * tableQuery.find({ userId: '123' }).subscribe(items => console.log(items))
   */
  find(where?: Partial<Model>): QueryBuilder<Schema, Model, typeof this.tableName> {
    return new QueryBuilder(this, this.tableName as any, where);
  }

  /**
   * Create one or more records
   * @example
   * tableQuery.create({ name: 'John', email: 'john@example.com' })
   * tableQuery.create([{ name: 'John' }, { name: 'Jane' }])
   */
  async create(
    data: Values<ModelPayload<Model>> | Values<ModelPayload<Model>>[]
  ): Promise<Model[]> {
    const result = await this.db.createLocal<Model>(this.table, data);
    return result as unknown as Model[];
  }

  /**
   * Delete records matching the given conditions
   * @example
   * tableQuery.delete({ status: 'archived' })
   */
  async delete(where: Partial<Model>): Promise<void> {
    const whereKeys = Object.keys(where);
    if (whereKeys.length === 0) {
      throw new Error(
        "Delete requires at least one condition in the where clause for safety"
      );
    }

    const whereClause = whereKeys
      .map((key) => `${key} = $${key}`)
      .join(" AND ");
    const query = `DELETE FROM ${this.tableName} WHERE ${whereClause};`;

    await this.db.queryLocal(query, where);
  }

  /**
   * Update records matching the where conditions with the provided update data
   * @example
   * tableQuery.update({
   *   where: { status: 'pending' },
   *   update: { status: 'approved' }
   * })
   */
  async update(options: {
    where: Partial<Model>;
    update: Partial<Model>;
  }): Promise<void> {
    const { where, update } = options;

    const whereKeys = Object.keys(where);
    if (whereKeys.length === 0) {
      throw new Error(
        "Update requires at least one condition in the where clause for safety"
      );
    }

    const updateKeys = Object.keys(update);
    if (updateKeys.length === 0) {
      throw new Error("Update requires at least one field to update");
    }

    const whereClause = whereKeys
      .map((key) => `${key} = $where_${key}`)
      .join(" AND ");
    const setClause = updateKeys
      .map((key) => `${key} = $update_${key}`)
      .join(", ");
    const query = `UPDATE ${this.tableName} SET ${setClause} WHERE ${whereClause};`;

    const vars: Record<string, unknown> = {};
    whereKeys.forEach((key) => {
      vars[`where_${key}`] = where[key as keyof Model];
    });
    updateKeys.forEach((key) => {
      vars[`update_${key}`] = update[key as keyof Model];
    });

    await this.db.queryLocal(query, vars);
  }

  async liveQuery(
    options: LiveQueryOptions<Model>,
    callback: (queryId: Model[]) => void
  ): Promise<LiveQueryList<Schema, Model>> {
    const liveQuery = new LiveQueryList<Schema, Model>(
      this.buildQuery("SELECT", options),
      this.tableName,
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
