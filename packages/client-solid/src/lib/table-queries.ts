import { Frame, RecordId, Surreal, Table, Values, DateTime } from "surrealdb";
import { proxy, ref } from "valtio";
import { GenericModel, GenericSchema, ModelPayload } from "./models";
import { QueryResponse, SyncedDb } from "..";

export interface LiveQueryOptions<Model extends GenericModel> {
  select?: ((keyof Model & string) | "*")[];
  where?: Partial<Model> | { id: RecordId };
  limit?: number;
  offset?: number;
}

export interface QueryOptions<Model extends GenericModel>
  extends LiveQueryOptions<Model> {
  orderBy?: Partial<Record<keyof Model, "asc" | "desc">>;
}

export interface QueryInfo {
  query: string;
  vars?: Record<string, unknown>;
}

/**
 * Recursively convert DateTime objects to Date objects and wrap RecordId objects with ref()
 */
function convertDateTimeToDate(value: any): any {
  // Convert DateTime to Date
  if (value instanceof DateTime) {
    return value.toDate();
  }
  // Wrap RecordId with ref() to prevent valtio proxying
  if (value instanceof RecordId) {
    return ref(value);
  }
  // Process arrays recursively
  if (Array.isArray(value)) {
    return value.map(convertDateTimeToDate);
  }
  // Process plain objects recursively
  if (value && typeof value === "object" && value.constructor === Object) {
    const result: any = {};
    for (const key in value) {
      if (Object.prototype.hasOwnProperty.call(value, key)) {
        result[key] = convertDateTimeToDate(value[key]);
      }
    }
    return result;
  }
  return value;
}

/**
 * Process models: convert DateTime to Date and wrap all RecordId objects with ref()
 */
function wrapModelIdsWithRef<Model extends GenericModel>(
  models: Model[]
): Model[] {
  return models.map((model) => {
    // Convert DateTime to Date and wrap RecordId with ref (including id and relationship fields)
    return convertDateTimeToDate(model);
  });
}

/**
 * Convert a SELECT query to a LIVE SELECT query
 * Ensures ORDER BY is removed as it's not supported in LIVE queries
 */
export function toLiveQuery(queryInfo: QueryInfo): QueryInfo {
  let query = queryInfo.query;

  // Remove ORDER BY clause if present (not supported in LIVE SELECT)
  query = query.replace(/\s+ORDER BY[^;]+/i, "");

  // Replace SELECT with LIVE SELECT
  query = query.replace(/^\s*SELECT\s+/i, "LIVE SELECT ");

  return {
    query,
    vars: queryInfo.vars,
  };
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
    // Wrap ids with ref() to prevent valtio from proxying RecordId objects
    const wrappedState = wrapModelIdsWithRef(newState);
    this.state.length = 0;
    this.state.push(...wrappedState);
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
    // Try to fetch from remote first to get the latest data
    const remote = this.db.getRemote();
    let models: Model[] = [];

    if (remote) {
      try {
        console.log("[LiveQueryList] Fetching initial data from remote...");
        const [remoteModels] = await this.db
          .queryRemote(this.hydrationQuery.query, this.hydrationQuery.vars)
          .collect();

        if (remoteModels && remoteModels.length > 0) {
          console.log("[LiveQueryList] Using remote data directly:", remoteModels.length, "items");
          models = remoteModels as Model[];
        } else {
          console.log("[LiveQueryList] No remote data found, falling back to local");
          const [localModels] = await this.db
            .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
            .collect();
          models = localModels as Model[];
        }
      } catch (error) {
        console.warn("[LiveQueryList] Failed to fetch from remote, falling back to local cache:", error);
        const [localModels] = await this.db
          .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
          .collect();
        models = localModels as Model[];
      }
    } else {
      // No remote connection, use local only
      console.log("[LiveQueryList] No remote connection, using local cache");
      const [localModels] = await this.db
        .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
        .collect();
      models = localModels as Model[];
    }

    // Wrap ids with ref() to prevent valtio from proxying RecordId objects
    this.state = wrapModelIdsWithRef(models);
    console.log("[LiveQueryList] Hydrated with", this.state.length, "items");
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
        // Re-fetch from remote when changes occur (instead of trying to update local cache)
        console.log("[LiveQueryList] Remote change detected, re-fetching data...");
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
  private options: QueryOptions<Model> = {};
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
   * Add ordering to the query (only for non-live queries)
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
  related<RelatedTable extends string>(relatedTable: RelatedTable): this {
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
  constructor(
    private db: SyncedDb<Schema>,
    public readonly tableName: string
  ) {}

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

    // Only add ORDER BY for non-live queries
    const orderClause =
      method === "SELECT" && "orderBy" in props
        ? Object.entries(props.orderBy ?? {})
            .map(([key, val]) => `${key} ${val}`)
            .join(", ")
        : "";

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
   * tableQuery.find({ status: 'active' }).limit(10).query()
   * tableQuery.find({ userId: '123' }).subscribe(items => console.log(items))
   */
  find(
    where?: Partial<Model>
  ): QueryBuilder<Schema, Model, typeof this.tableName> {
    return new QueryBuilder(this, this.tableName as any, where);
  }

  /**
   * Create one or more records
   * Creates in local DB first, then syncs to remote if available.
   * If remote creation fails, rolls back the local creation.
   * @example
   * tableQuery.create({ name: 'John', email: 'john@example.com' })
   * tableQuery.create([{ name: 'John' }, { name: 'Jane' }])
   */
  async create(
    data: Values<ModelPayload<Model>> | Values<ModelPayload<Model>>[]
  ): Promise<Model[]> {
    console.log("[TableQuery.create] Creating records", data);

    // Create locally first
    const localResult = await this.db.createLocal<Model>(this.tableName, data);
    const models = localResult as unknown as Model[];
    console.log("[TableQuery.create] Local creation successful", models);

    // Try to sync to remote if available
    const remote = this.db.getRemote();
    if (remote) {
      try {
        console.log("[TableQuery.create] Syncing to remote...");
        await this.db.createRemote<Model>(this.tableName, data);
        console.log("[TableQuery.create] Remote creation successful");
      } catch (error) {
        console.error(
          "[TableQuery.create] Remote creation failed, rolling back local changes",
          error
        );

        // Rollback: delete the locally created records
        try {
          for (const model of models) {
            if (model.id && model.id instanceof RecordId) {
              await this.db.getLocal().delete(model.id);
              console.log(
                "[TableQuery.create] Rolled back local record",
                model.id
              );
            }
          }
        } catch (rollbackError) {
          console.error("[TableQuery.create] Rollback failed", rollbackError);
        }

        // Re-throw the original error
        throw new Error(
          `Failed to create records: ${
            error instanceof Error ? error.message : "Remote creation failed"
          }`
        );
      }
    }

    // Wrap ids with ref() to prevent valtio from proxying RecordId objects
    return wrapModelIdsWithRef(models);
  }

  /**
   * Delete records matching the given conditions
   * Deletes from local DB first, then syncs to remote if available.
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

    console.log("[TableQuery.delete] Deleting records from local", where);
    await this.db.queryLocal(query, where);

    // Sync to remote if available
    const remote = this.db.getRemote();
    if (remote) {
      try {
        console.log("[TableQuery.delete] Syncing deletion to remote...");
        await this.db.queryRemote(query, where);
        console.log("[TableQuery.delete] Remote deletion successful");
      } catch (error) {
        console.error(
          "[TableQuery.delete] Remote deletion failed (continuing anyway)",
          error
        );
        // Note: We don't rollback deletes as they're already gone locally
      }
    }
  }

  /**
   * Update records matching the where conditions with the provided update data
   * Updates local DB first, then syncs to remote if available.
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

    console.log("[TableQuery.update] Updating records in local", {
      where,
      update,
    });
    await this.db.queryLocal(query, vars);

    // Sync to remote if available
    const remote = this.db.getRemote();
    if (remote) {
      try {
        console.log("[TableQuery.update] Syncing update to remote...");
        await this.db.queryRemote(query, vars);
        console.log("[TableQuery.update] Remote update successful");
      } catch (error) {
        console.error(
          "[TableQuery.update] Remote update failed (continuing anyway)",
          error
        );
        // Note: We don't rollback updates as they're already applied locally
      }
    }
  }

  async liveQuery(
    options: LiveQueryOptions<Model>,
    callback: (queryId: Model[]) => void
  ): Promise<LiveQueryList<Schema, Model>> {
    // Strip out orderBy for live queries as it's not supported
    // Cast to any first to access potential orderBy, then create clean LiveQueryOptions
    const liveOptions: LiveQueryOptions<Model> = {
      select: options.select,
      where: options.where,
      limit: options.limit,
      offset: options.offset,
    };

    const liveQuery = new LiveQueryList<Schema, Model>(
      this.buildQuery("SELECT", liveOptions),
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
    return this.db.createLocal<Model>(this.tableName, data);
  }

  createRemote(
    data: Values<ModelPayload<Model>> | Values<ModelPayload<Model>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createRemote<Model>(this.tableName, data);
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

  deleteLocal(recordId: RecordId): Promise<any> {
    return this.db.deleteLocal<Model>(recordId);
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
