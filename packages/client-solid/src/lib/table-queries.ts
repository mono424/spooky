import { Frame, RecordId, Surreal, Table, Values, DateTime } from "surrealdb";
import { proxy } from "valtio";
import { GenericModel, GenericSchema, Model, ModelPayload } from "./models";
import {
  QueryBuilder as BaseQueryBuilder,
  buildQueryFromOptions,
  type QueryInfo,
  type LiveQueryOptions,
  type GetRelationshipFields,
  type WithRelated,
  type RelationshipsMetadata,
  type QueryModifier,
} from "@spooky/query-builder";
import { QueryResponse, SyncedDb } from "..";

// Re-export QueryInfo for internal use
export type { QueryInfo } from "@spooky/query-builder";

/**
 * Recursively convert DateTime objects to Date objects and RecordId objects to strings
 */
function convertDateTimeToDate(value: any): any {
  // Convert DateTime to Date
  if (value instanceof DateTime) {
    return value.toDate();
  }
  // Convert RecordId to string
  if (value instanceof RecordId) {
    return value.toString();
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
 * Process models: convert DateTime to Date and RecordId objects to strings
 */
function wrapModelIdsWithRef<Model extends GenericModel>(
  models: Model[]
): Model[] {
  return models.map((model) => {
    // Convert DateTime to Date and RecordId to string (including id and relationship fields)
    return convertDateTimeToDate(model);
  });
}

/**
 * Reactive query result with live updates
 */
export class ReactiveQueryResult<SModel extends GenericModel> {
  private state: SModel[];
  private liveQuery: LiveQueryList<any, SModel> | null = null;

  constructor() {
    this.state = proxy([]) as SModel[];
  }

  get data(): SModel[] {
    return this.state;
  }

  _setLiveQuery(liveQuery: LiveQueryList<any, SModel>): void {
    this.liveQuery = liveQuery;
  }

  _updateState(newState: SModel[]): void {
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

/**
 * Reactive query result for single record queries with live updates
 */
export class ReactiveQueryResultOne<SModel extends GenericModel> {
  public state: { value: SModel | null };
  private liveQuery: LiveQueryList<any, SModel> | null = null;

  // Type brand to help with type inference
  readonly __model!: SModel;

  constructor() {
    this.state = proxy({ value: null }) as { value: Model<SModel> | null };
  }

  get data(): { value: Model<SModel> | null } {
    return this.state;
  }

  _setLiveQuery(liveQuery: LiveQueryList<any, SModel>): void {
    this.liveQuery = liveQuery;
  }

  _updateState(newState: Model<SModel>[]): void {
    // Extract first element or set to null
    // Wrap ids with ref() to prevent valtio from proxying RecordId objects
    const wrappedState = wrapModelIdsWithRef(newState);
    this.state.value = wrappedState.length > 0 ? wrappedState[0] : null;
  }

  /**
   * Stop listening to live updates
   */
  kill(): void {
    this.liveQuery?.kill();
  }
}

/**
 * Helper type to extract the Model type from a ReactiveQueryResult
 * For ReactiveQueryResultOne, returns the model type or null
 * For ReactiveQueryResult, returns the array of models
 */
export type InferQueryModel<T> = T extends ReactiveQueryResultOne<infer M>
  ? M | null
  : T extends ReactiveQueryResult<infer M>
  ? readonly M[]
  : never;

export class LiveQueryList<
  Schema extends GenericSchema,
  SModel extends GenericModel
> {
  private state: Model<SModel>[];
  private unsubscribe: (() => void) | undefined;

  constructor(
    private liveQuery: QueryInfo,
    private hydrationQuery: QueryInfo,
    private tableName: string,
    private db: SyncedDb<Schema>,
    private callback: (items: Model<SModel>[]) => void
  ) {
    this.state = [];
    this.liveQuery = liveQuery;
    this.hydrationQuery = hydrationQuery;
    this.tableName = tableName;
    this.db = db;
    this.callback = callback;
  }

  private async hydrate(): Promise<void> {
    // Try to fetch from remote first to get the latest data
    const remote = this.db.getRemote();
    let models: Model<SModel>[] = [];

    if (remote) {
      try {
        console.log("[LiveQueryList] Fetching initial data from remote...");
        const [remoteModels] = await this.db
          .queryRemote(this.hydrationQuery.query, this.hydrationQuery.vars)
          .collect();

        if (remoteModels && remoteModels.length > 0) {
          console.log(
            "[LiveQueryList] Using remote data directly:",
            remoteModels.length,
            "items",
            remoteModels
          );
          models = remoteModels as Model<SModel>[];
        } else {
          console.log(
            "[LiveQueryList] No remote data found, falling back to local"
          );
          const [localModels] = await this.db
            .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
            .collect();
          models = localModels as Model<SModel>[];
        }
      } catch (error) {
        console.warn(
          "[LiveQueryList] Failed to fetch from remote, falling back to local cache:",
          error
        );
        const [localModels] = await this.db
          .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
          .collect();
        models = localModels as Model<SModel>[];
      }
    } else {
      // No remote connection, use local only
      console.log("[LiveQueryList] No remote connection, using local cache");
      const [localModels] = await this.db
        .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
        .collect();
      models = localModels as Model<SModel>[];
    }

    // Wrap ids with ref() to prevent valtio from proxying RecordId objects
    this.state = wrapModelIdsWithRef(models);
    console.log(
      "[LiveQueryList] Hydrated with",
      this.state.length,
      "items",
      this.state
    );
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
      this.liveQuery,
      [this.tableName],
      async () => {
        // Re-fetch from remote when changes occur (instead of trying to update local cache)
        console.log(
          "[LiveQueryList] Remote change detected, re-fetching data..."
        );
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

// Re-export WithRelated type for backward compatibility
export type { WithRelated } from "@spooky/query-builder";

/**
 * Fluent query builder for constructing queries with chainable methods
 * Extends the base query builder and adds Solid-specific reactive query execution
 */
export class QueryBuilder<
  Schema extends GenericSchema,
  SModel extends Record<string, any>,
  TableName extends keyof Schema & string,
  Relationships = any
> extends BaseQueryBuilder<Schema, SModel, TableName, Relationships> {
  constructor(
    private tableQuery: TableQuery<Schema, SModel, TableName, Relationships>,
    tableName: TableName,
    relationships?: RelationshipsMetadata,
    where?: Partial<Model<SModel>>
  ) {
    super(tableName, relationships, where);
  }

  /**
   * Override related to return the Solid-specific QueryBuilder type
   */
  related<
    RelatedField extends GetRelationshipFields<TableName, Relationships> &
      string
  >(
    relatedField: RelatedField,
    modifier?: QueryModifier<any>
  ): QueryBuilder<
    Schema,
    WithRelated<Schema, SModel, TableName, RelatedField, Relationships>,
    TableName,
    Relationships
  > {
    super.related(relatedField, modifier);
    return this as any;
  }

  /**
   * Override chainable methods to return the Solid-specific QueryBuilder type
   */
  where(conditions: Partial<Model<SModel>>): this {
    super.where(conditions);
    return this;
  }

  select(...fields: ((keyof SModel & string) | "*")[]): this {
    super.select(...fields);
    return this;
  }

  orderBy(
    field: keyof SModel & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    super.orderBy(field, direction);
    return this;
  }

  limit(count: number): this {
    super.limit(count);
    return this;
  }

  offset(count: number): this {
    super.offset(count);
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
  query(): ReactiveQueryResult<SModel> {
    const result = new ReactiveQueryResult<SModel>();

    (async () => {
      const liveQuery = await this.tableQuery.liveQuery(
        this.getOptions(),
        (items) => {
          result._updateState(items);
        }
      );

      result._setLiveQuery(liveQuery);
    })();

    return result;
  }

  /**
   * Execute the query and return a single reactive record that updates automatically
   * Automatically sets limit to 1 and returns a single record instead of an array
   * @example
   * const result = tableQuery.find({ id: new RecordId('thread', '123') }).one();
   * // result.data is a single Model or null (not an array)
   * console.log(result.data); // access the single reactive record
   *
   * // Clean up when done
   * result.kill();
   */
  one(): ReactiveQueryResultOne<SModel> {
    // Automatically set limit to 1
    this.limit(1);

    const result = new ReactiveQueryResultOne<SModel>();

    (async () => {
      const liveQuery = await this.tableQuery.liveQuery(
        this.getOptions(),
        (items) => {
          result._updateState(items);
        }
      );

      result._setLiveQuery(liveQuery);
    })();

    return result;
  }
}

/**
 * Table query interface for a specific table. The response type is inferred from Schema[K].
 */
export class TableQuery<
  Schema extends GenericSchema,
  SModel extends Record<string, any>,
  TableName extends keyof Schema & string,
  Relationships = any
> {
  constructor(
    private db: SyncedDb<Schema, Relationships>,
    public readonly tableName: TableName
  ) {}

  /**
   * Build a query using the query-builder package
   */
  private buildQuery(
    method: "LIVE SELECT" | "SELECT",
    props: LiveQueryOptions<SModel>
  ): QueryInfo {
    // Get relationships metadata from db config
    const relationships = (this.db as any).config?.relationships;

    // Use buildQueryFromOptions from query-builder package
    return buildQueryFromOptions(
      method,
      this.tableName,
      props as any,
      relationships
    );
  }

  /**
   * Start a fluent query with optional where conditions
   * @example
   * tableQuery.find({ status: 'active' }).limit(10).query()
   * tableQuery.find({ userId: '123' }).subscribe(items => console.log(items))
   */
  find(
    where?: Partial<Model<SModel>>
  ): QueryBuilder<Schema, SModel, TableName, Relationships> {
    // Get relationships metadata from db config
    const relationships = (this.db as any).config?.relationships;
    return new QueryBuilder<Schema, SModel, TableName, Relationships>(
      this,
      this.tableName,
      relationships,
      where
    );
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
    data: Values<ModelPayload<SModel>> | Values<ModelPayload<SModel>>[]
  ): Promise<SModel[]> {
    console.log("[TableQuery.create] Creating records", data);

    // Create locally first
    const localResult = await this.db.createLocal<SModel>(this.tableName, data);
    const models = localResult as unknown as SModel[];
    console.log("[TableQuery.create] Local creation successful", models);

    // Try to sync to remote if available
    const remote = this.db.getRemote();
    if (remote) {
      try {
        console.log("[TableQuery.create] Syncing to remote...");
        await this.db.createRemote<SModel>(this.tableName, data);
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
  async delete(where: Partial<SModel>): Promise<void> {
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
    where: Partial<SModel>;
    update: Partial<SModel>;
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
      vars[`where_${key}`] = where[key as keyof SModel];
    });
    updateKeys.forEach((key) => {
      vars[`update_${key}`] = update[key as keyof SModel];
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
    options: LiveQueryOptions<SModel>,
    callback: (items: Model<SModel>[]) => void
  ): Promise<LiveQueryList<Schema, SModel>> {
    // Build LIVE SELECT query directly (no ORDER BY, LIMIT, or START)
    const liveQueryInfo = this.buildQuery("LIVE SELECT", options);
    const selectQuery = this.buildQuery("SELECT", options);

    console.log("[TableQuery.liveQuery] liveQueryInfo", {
      liveQueryInfo,
      selectQuery,
    });

    const liveQuery = new LiveQueryList<Schema, SModel>(
      liveQueryInfo,
      selectQuery,
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
  ): QueryResponse<SModel> {
    return this.db.queryLocal<SModel>(sql, vars);
  }

  queryRemote(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<SModel> {
    return this.db.queryRemote<SModel>(sql, vars);
  }

  createLocal(
    data: Values<ModelPayload<SModel>> | Values<ModelPayload<SModel>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createLocal<SModel>(this.tableName, data);
  }

  createRemote(
    data: Values<ModelPayload<SModel>> | Values<ModelPayload<SModel>>[]
  ): ReturnType<Surreal["insert"]> {
    return this.db.createRemote<SModel>(this.tableName, data);
  }

  updateLocal(
    recordId: RecordId,
    data: Partial<SModel>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateLocal<SModel>(recordId, data);
  }

  updateRemote(
    recordId: RecordId,
    data: Partial<SModel>
  ): ReturnType<Surreal["update"]> {
    return this.db.updateRemote<SModel>(recordId, data);
  }

  deleteLocal(recordId: RecordId): Promise<any> {
    return this.db.deleteLocal<SModel>(recordId);
  }

  deleteRemote(table: Table): ReturnType<Surreal["delete"]> {
    return this.db.deleteRemote<SModel>(table);
  }
}

/**
 * Query namespace that provides table-scoped query access
 */
class QueryNamespaceImpl<
  Schema extends GenericSchema,
  Relationships = any
> {
  private tableCache = new Map<
    string,
    TableQuery<Schema, any, any, Relationships>
  >();

  constructor(private db: SyncedDb<Schema, Relationships>) {}

  getTable<K extends keyof Schema & string>(
    key: K
  ): TableQuery<Schema, NonNullable<Schema[K]>, K, Relationships> {
    if (!this.tableCache.has(key)) {
      this.tableCache.set(
        key,
        new TableQuery<Schema, NonNullable<Schema[K]>, K, Relationships>(
          this.db,
          key
        )
      );
    }
    return this.tableCache.get(key) as TableQuery<
      Schema,
      NonNullable<Schema[K]>,
      K,
      Relationships
    >;
  }
}

export class QueryNamespace<
  Schema extends GenericSchema,
  Relationships = any
> {
  constructor(db: SyncedDb<Schema, Relationships>) {
    const impl = new QueryNamespaceImpl(db);
    // Create a proxy to handle dynamic table access
    return new Proxy(impl, {
      get(target, prop: keyof Schema | string | symbol) {
        if (typeof prop === "string") {
          const key = prop as keyof Schema & string;
          return target.getTable(key);
        }
        return Reflect.get(target, prop);
      },
    });
  }
}

/**
 * Type helper for table queries
 */
export type TableQueries<
  Schema extends GenericSchema,
  Relationships = any
> = {
  [K in keyof Schema & string]: TableQuery<
    Schema,
    NonNullable<Schema[K]>,
    K,
    Relationships
  >;
};
