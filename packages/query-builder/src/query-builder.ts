import { RecordId } from "surrealdb";
import type {
  GenericModel,
  QueryInfo,
  QueryOptions,
  QueryModifier,
  QueryModifierBuilder,
  RelatedQuery,
  SchemaAwareQueryModifier,
  SchemaAwareQueryModifierBuilder,
} from "./types";
import type {
  TableNames,
  GetTable,
  TableModel,
  TableRelationships,
  GetRelationship,
  SchemaStructure,
  TableFieldNames,
  ColumnSchema,
} from "./table-schema";

/**
 * Parse a string ID to RecordId
 * - If it's in the format "table:id", use it as-is
 * - If it's just an ID without ":", prepend the table name
 * @param value - The value to parse (could be a string ID)
 * @param tableName - The table name to use if the ID doesn't contain ":"
 * @param fieldName - The field name to determine if this is an ID field
 */
function parseStringToRecordId(
  value: unknown,
  tableName?: string,
  fieldName?: string
): unknown {
  if (typeof value !== "string") return value;

  // If it already contains ":", parse it as a full record ID
  if (value.includes(":")) {
    const [table, ...idParts] = value.split(":");
    const id = idParts.join(":"); // Handle IDs that contain colons
    return new RecordId(table, id);
  }

  // If this is an "id" field and we have a table name, prepend it
  if (fieldName === "id" && tableName) {
    return new RecordId(tableName, value);
  }

  // Otherwise, return as-is (it might not be an ID at all)
  return value;
}

/**
 * Recursively parse string IDs to RecordId in an object
 * @param obj - The object to parse
 * @param tableName - The table name to use for ID fields without ":"
 */
function parseObjectIdsToRecordId(obj: unknown, tableName?: string): unknown {
  if (obj === null || obj === undefined) return obj;

  if (typeof obj === "string") {
    return parseStringToRecordId(obj, tableName);
  }

  if (Array.isArray(obj)) {
    return obj.map((item) => parseObjectIdsToRecordId(item, tableName));
  }

  if (typeof obj === "object" && obj.constructor === Object) {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj)) {
      // Parse recursively, passing the field name to identify ID fields
      result[key] =
        typeof value === "string"
          ? parseStringToRecordId(value, tableName, key)
          : parseObjectIdsToRecordId(value, tableName);
    }
    return result;
  }

  return obj;
}

export type Executor<T extends { columns: Record<string, ColumnSchema> }> = (
  query: InnerQuery<T, boolean>
) => { cleanup: () => void };

type InnerQueryListener<T extends { columns: Record<string, ColumnSchema> }> = {
  callback: (data: TableModel<T>[]) => void;
};

export type ReactiveQueryResult<
  T extends { columns: Record<string, ColumnSchema> }
> = {
  data: TableModel<T>[];
  hash: number;
  subscribe: (callback: (data: TableModel<T>[]) => void) => () => void;
  unsubscribeAll: () => void;
  kill: () => void;
};

export class InnerQuery<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
> {
  private _hash: number;
  private _data: TableModel<T>[];
  private _listeners: InnerQueryListener<T>[] = [];
  private _selectQuery: QueryInfo;
  private _selectLiveQuery: QueryInfo;
  private _subqueries: InnerQuery<
    { columns: Record<string, ColumnSchema> },
    boolean
  >[];

  private _hasRun: boolean = false;
  private _unsubs: (() => void)[] = [];
  private _cleanup: () => void = () => {};

  constructor(
    private readonly tableName: string,
    private readonly options: QueryOptions<TableModel<T>, IsOne>,
    private readonly schema: SchemaStructure,
    private readonly executor: Executor<any>
  ) {
    this._data = [];
    this._listeners = [];

    this._selectQuery = buildQueryFromOptions(
      "SELECT",
      this.tableName,
      this.options,
      this.schema
    );

    this._hash = this._selectQuery.hash;

    this._selectLiveQuery = buildQueryFromOptions(
      "LIVE SELECT DIFF",
      this.tableName,
      this.options,
      this.schema
    );

    this._subqueries = extractSubqueryQueryInfos(
      schema,
      this.options,
      this.executor
    );
  }

  get subqueries(): InnerQuery<
    { columns: Record<string, ColumnSchema> },
    boolean
  >[] {
    return this._subqueries;
  }

  get selectQuery(): QueryInfo {
    return this._selectQuery;
  }

  get selectLiveQuery(): QueryInfo {
    return this._selectLiveQuery;
  }

  get hash(): number {
    return this._hash;
  }

  get isOne(): boolean {
    return this.options.isOne ?? false;
  }

  get data(): TableModel<T>[] {
    return this._data;
  }

  public setData(data: TableModel<T>[]): void {
    this._data = data;
    this._listeners.forEach(({ callback }) => callback(data));
  }

  private addListener(listener: InnerQueryListener<T>): () => void {
    this._listeners.push(listener);
    return () => {
      this._listeners = this._listeners.filter((l) => l !== listener);
    };
  }

  public subscribe(callback: (data: TableModel<T>[]) => void): () => void {
    const unsub = this.addListener({ callback });
    this._unsubs.push(unsub);
    return unsub;
  }

  public unsubscribeAll(): void {
    this._unsubs.forEach((unsub) => unsub());
    this._unsubs = [];
  }

  public kill(): void {
    this.unsubscribeAll();
    this._cleanup?.();
  }

  private getReactiveQueryResult(): ReactiveQueryResult<T> {
    return {
      data: this.data,
      hash: this.hash,
      subscribe: (callback: (data: TableModel<T>[]) => void) =>
        this.subscribe(callback),
      unsubscribeAll: () => this.unsubscribeAll(),
      kill: () => this.kill(),
    };
  }

  public run(): ReactiveQueryResult<T> {
    if (this._hasRun) {
      throw new Error("Query has already been run");
    }
    this._hasRun = true;

    const { cleanup } = this.executor(this);
    this._cleanup = cleanup;
    return this.getReactiveQueryResult();
  }
}

/**
 * Helper type to get the model type for a related table
 */
type GetRelatedModel<
  S extends SchemaStructure,
  RelatedTableName extends string
> = RelatedTableName extends TableNames<S>
  ? TableModel<GetTable<S, RelatedTableName>>
  : never;

/**
 * Helper type to build the related fields object based on accumulated relationships
 */
type BuildRelatedFields<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  RelatedFields extends Record<string, any>
> = {
  [K in keyof RelatedFields]: RelatedFields[K] extends { cardinality: "one" }
    ? GetRelatedModel<S, RelatedFields[K]["to"]> | null
    : GetRelatedModel<S, RelatedFields[K]["to"]>[];
};

/**
 * The final result type combining base model with related fields
 */
type QueryResult<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  RelatedFields extends Record<string, any>,
  IsOne extends boolean
> = IsOne extends true
  ?
      | (TableModel<GetTable<S, TableName>> &
          BuildRelatedFields<S, TableName, RelatedFields>)
      | null
  : (TableModel<GetTable<S, TableName>> &
      BuildRelatedFields<S, TableName, RelatedFields>)[];

export class FinalQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean
> {
  private _innerQuery: InnerQuery<T, IsOne>;

  constructor(
    private readonly tableName: TableName,
    private readonly options: QueryOptions<TableModel<T>, IsOne>,
    private readonly schema: S,
    private readonly executor: Executor<T>
  ) {
    this._innerQuery = new InnerQuery<T, IsOne>(
      this.tableName,
      this.options,
      this.schema,
      this.executor
    );
  }

  get hash(): number {
    return this._innerQuery.hash;
  }

  get data(): QueryResult<S, TableName, RelatedFields, IsOne> {
    return this._innerQuery.data as any;
  }

  get isOne(): boolean {
    return this.options.isOne ?? false;
  }

  select() {
    return {
      ...this._innerQuery.run(),
      data: this.data,
    };
  }
}

/**
 * Schema-aware query modifier builder implementation
 * This version provides full type safety for nested relationships
 */
class SchemaAwareQueryModifierBuilderImpl<
  S extends SchemaStructure,
  TableName extends TableNames<S>
> implements SchemaAwareQueryModifierBuilder<S, TableName>
{
  private options: QueryOptions<TableModel<GetTable<S, TableName>>, boolean> =
    {};

  constructor(
    private readonly tableName: TableName,
    private readonly schema: S
  ) {}

  where(conditions: Partial<TableModel<GetTable<S, TableName>>>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  select(
    ...fields: ((keyof TableModel<GetTable<S, TableName>> & string) | "*")[]
  ): this {
    if (this.options.select) {
      throw new Error("Select can only be called once per query");
    }
    this.options.select = fields;
    return this;
  }

  limit(count: number): this {
    this.options.limit = count;
    return this;
  }

  offset(count: number): this {
    this.options.offset = count;
    return this;
  }

  orderBy(
    field: keyof TableModel<GetTable<S, TableName>> & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<
      Record<keyof TableModel<GetTable<S, TableName>>, "asc" | "desc">
    >;
    return this;
  }

  // Schema-aware implementation for nested relationships with full type inference
  related<
    Field extends TableRelationships<S, TableName>["field"],
    Rel extends GetRelationship<S, TableName, Field>
  >(
    relatedField: Field,
    modifier?: SchemaAwareQueryModifier<S, Rel["to"]>
  ): this {
    if (!this.options.related) {
      this.options.related = [];
    }

    const exists = this.options.related.some(
      (r) => (r.alias || r.relatedTable) === relatedField
    );

    if (!exists) {
      // Look up the relationship from schema
      const relationship = this.schema.relationships.find(
        (r) => r.from === this.tableName && r.field === relatedField
      );

      if (!relationship) {
        throw new Error(
          `Relationship '${String(relatedField)}' not found for table '${
            this.tableName
          }'`
        );
      }

      const relatedTable = relationship.to;
      const cardinality = relationship.cardinality;
      const foreignKeyField =
        cardinality === "many" ? this.tableName : relatedField;

      this.options.related.push({
        relatedTable,
        alias: relatedField as string,
        modifier: modifier as QueryModifier<GenericModel>,
        cardinality,
        foreignKeyField: foreignKeyField as string,
      } as RelatedQuery & { foreignKeyField: string });
    }
    return this;
  }

  _getOptions(): QueryOptions<TableModel<GetTable<S, TableName>>, boolean> {
    return this.options;
  }
}

/**
 * Fluent query builder for constructing queries with chainable methods
 * Now with full type inference from schema constant AND related field accumulation!
 */
export class QueryBuilder<
  const S extends SchemaStructure,
  const TableName extends TableNames<S>,
  const RelatedFields extends Record<string, any> = {},
  const IsOne extends boolean = false
> {
  constructor(
    private readonly schema: S,
    private readonly tableName: TableName,
    private readonly executer: Executor<GetTable<S, TableName>>,
    private options: QueryOptions<
      TableModel<GetTable<S, TableName>>,
      IsOne
    > = {}
  ) {}

  /**
   * Add additional where conditions
   */
  where(
    conditions: Partial<TableModel<GetTable<S, TableName>>>
  ): QueryBuilder<S, TableName, RelatedFields, IsOne> {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  /**
   * Specify fields to select
   */
  select(
    ...fields: ((keyof TableModel<GetTable<S, TableName>> & string) | "*")[]
  ): QueryBuilder<S, TableName, RelatedFields, IsOne> {
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
    field: TableFieldNames<GetTable<S, TableName>>,
    direction: "asc" | "desc" = "asc"
  ): QueryBuilder<S, TableName, RelatedFields, IsOne> {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<
      Record<keyof TableModel<GetTable<S, TableName>>, "asc" | "desc">
    >;
    return this;
  }

  /**
   * Add limit to the query (only for non-live queries)
   */
  limit(count: number): QueryBuilder<S, TableName, RelatedFields, IsOne> {
    this.options.limit = count;
    return this;
  }

  /**
   * Add offset to the query (only for non-live queries)
   */
  offset(count: number): QueryBuilder<S, TableName, RelatedFields, IsOne> {
    this.options.offset = count;
    return this;
  }

  one(): QueryBuilder<S, TableName, RelatedFields, true> {
    return new QueryBuilder<S, TableName, RelatedFields, true>(
      this.schema,
      this.tableName,
      this.executer,
      { ...this.options, isOne: true }
    );
  }

  /**
   * Include related data via subqueries
   * Field and cardinality are validated against schema relationships
   * Now accumulates the related field in the type!
   */
  related<
    Field extends TableRelationships<S, TableName>["field"],
    Rel extends GetRelationship<S, TableName, Field>
  >(
    field: Field,
    modifierOrCardinality?:
      | SchemaAwareQueryModifier<S, Rel["to"]>
      | Rel["cardinality"],
    modifier?: SchemaAwareQueryModifier<S, Rel["to"]>
  ): QueryBuilder<
    S,
    TableName,
    RelatedFields & {
      [K in Field]: {
        to: Rel["to"];
        cardinality: Rel["cardinality"];
      };
    },
    IsOne
  > {
    if (!this.options.related) {
      this.options.related = [];
    }

    // Check if field already exists
    const exists = this.options.related.some(
      (r) => (r.alias || r.relatedTable) === field
    );

    if (exists) {
      return this as any;
    }

    // Look up relationship metadata from schema
    const relationship = this.schema.relationships.find(
      (r) => r.from === this.tableName && r.field === field
    );

    if (!relationship) {
      throw new Error(
        `Relationship '${String(field)}' not found for table '${
          this.tableName
        }'`
      );
    }

    // Determine cardinality and modifier based on arguments
    let actualCardinality: "one" | "many";
    let actualModifier: SchemaAwareQueryModifier<S, Rel["to"]> | undefined;

    if (typeof modifierOrCardinality === "function") {
      // Signature: related(field, modifier)
      actualCardinality = relationship.cardinality;
      actualModifier = modifierOrCardinality;
    } else if (
      modifierOrCardinality === "one" ||
      modifierOrCardinality === "many"
    ) {
      // Signature: related(field, cardinality, modifier)
      actualCardinality = modifierOrCardinality;
      actualModifier = modifier;
    } else {
      // Signature: related(field)
      actualCardinality = relationship.cardinality;
      actualModifier = undefined;
    }

    // Determine foreign key field based on cardinality
    const foreignKeyField =
      actualCardinality === "many" ? this.tableName : field;

    // Cast the schema-aware modifier to the runtime type
    // At runtime, QueryModifierBuilderImpl will work correctly with the schema
    const wrappedModifier = actualModifier as
      | QueryModifier<GenericModel>
      | undefined;

    this.options.related.push({
      relatedTable: relationship.to,
      alias: field as string,
      modifier: wrappedModifier,
      cardinality: actualCardinality,
      foreignKeyField: foreignKeyField as string,
    } as RelatedQuery & { foreignKeyField: string });

    return this as any;
  }

  /**
   * Get the current query options
   */
  getOptions(): QueryOptions<TableModel<GetTable<S, TableName>>, IsOne> {
    return this.options;
  }

  /**
   * Build query methods for SELECT and LIVE SELECT
   * @returns Object with select() and selectLive() methods
   */
  build(): FinalQuery<
    S,
    TableName,
    GetTable<S, TableName>,
    RelatedFields,
    IsOne
  > {
    return new FinalQuery<
      S,
      TableName,
      GetTable<S, TableName>,
      RelatedFields,
      IsOne
    >(this.tableName, this.options, this.schema, this.executer);
  }
}

function cyrb53(str: string, seed: number): number {
  let h1 = 0xdeadbeef ^ seed,
    h2 = 0x41c6ce57 ^ seed;
  for (let i = 0, ch; i < str.length; i++) {
    ch = str.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507);
  h1 ^= Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507);
  h2 ^= Math.imul(h1 ^ (h1 >>> 13), 3266489909);

  return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

export function extractSubqueryQueryInfos<S extends SchemaStructure>(
  schema: S,
  options: QueryOptions<GenericModel, boolean>,
  executer: Executor<{ columns: Record<string, ColumnSchema> }>
): InnerQuery<{ columns: Record<string, ColumnSchema> }, boolean>[] {
  if (!options.related) {
    return [];
  }

  return options.related.map(
    (rel) =>
      new InnerQuery(
        rel.relatedTable,
        rel
          .modifier?.(
            new SchemaAwareQueryModifierBuilderImpl(rel.relatedTable, schema)
          )
          ._getOptions() ?? {},
        schema,
        executer
      )
  );
}

/**
 * Build a query string from query options
 * @param method - The query method (SELECT or LIVE SELECT)
 * @param tableName - The table name to query
 * @param options - The query options (where, select, orderBy, etc.)
 * @param schema - Optional schema for resolving nested relationships
 * @returns QueryInfo with the generated SQL and variables
 */
export function buildQueryFromOptions<
  TModel extends GenericModel,
  IsOne extends boolean
>(
  method: "SELECT" | "LIVE SELECT" | "LIVE SELECT DIFF",
  tableName: string,
  options: QueryOptions<TModel, IsOne>,
  schema: SchemaStructure
): QueryInfo {
  if (options.isOne) {
    options.limit = 1;
  }
  const isLiveQuery = method === "LIVE SELECT" || method === "LIVE SELECT DIFF";

  // Parse where conditions to convert string IDs to RecordId
  const parsedWhere = options.where
    ? parseObjectIdsToRecordId(options.where, tableName)
    : undefined;

  // Build SELECT clause
  let selectClause = "*";
  if (options.select && options.select.length > 0) {
    selectClause = options.select.join(", ");
  }

  // Build related subqueries (fetch clauses)
  let fetchClauses = "";
  if (!isLiveQuery && options.related && options.related.length > 0) {
    const subqueries = options.related.map((rel) => buildSubquery(rel, schema));
    fetchClauses = ", " + subqueries.join(", ");
  }

  // Start building the query
  let query = `${method} ${selectClause}${fetchClauses} FROM ${tableName}`;

  // Build WHERE clause
  const vars: Record<string, unknown> = {};
  if (parsedWhere && Object.keys(parsedWhere).length > 0) {
    const conditions: string[] = [];
    for (const [key, value] of Object.entries(parsedWhere)) {
      const varName = key;
      vars[varName] = value;
      conditions.push(`${key} = $${varName}`);
    }
    query += ` WHERE ${conditions.join(" AND ")}`;
  }

  // Add ORDER BY, LIMIT, START only for non-live queries
  if (!isLiveQuery) {
    if (options.orderBy && Object.keys(options.orderBy).length > 0) {
      const orderClauses = Object.entries(options.orderBy).map(
        ([field, direction]) => `${field} ${direction}`
      );
      query += ` ORDER BY ${orderClauses.join(", ")}`;
    }

    if (options.limit !== undefined) {
      query += ` LIMIT ${options.limit}`;
    }

    if (options.offset !== undefined) {
      query += ` START ${options.offset}`;
    }
  }

  query += ";";

  console.log(`[buildQuery] Generated ${method} query:`, query);
  console.log(`[buildQuery] Query vars:`, vars);

  return {
    query,
    hash: cyrb53(query, 0),
    vars: Object.keys(vars).length > 0 ? vars : undefined,
  };
}

/**
 * Build a subquery for a related field
 */
function buildSubquery(
  rel: RelatedQuery & { foreignKeyField?: string },
  schema: SchemaStructure
): string {
  const { relatedTable, alias, modifier, cardinality } = rel;
  const foreignKeyField = rel.foreignKeyField || alias;

  let subquerySelect = "*";
  let subqueryWhere = "";
  let subqueryOrderBy = "";
  let subqueryLimit = "";

  // If there's a modifier, apply it to get the sub-options
  if (modifier) {
    const modifierBuilder = new SchemaAwareQueryModifierBuilderImpl(
      relatedTable,
      schema
    );
    modifier(modifierBuilder);
    const subOptions = modifierBuilder._getOptions();

    // Build sub-select
    if (subOptions.select && subOptions.select.length > 0) {
      subquerySelect = subOptions.select.join(", ");
    }

    // Build sub-where
    if (subOptions.where && Object.keys(subOptions.where).length > 0) {
      const parsedSubWhere = parseObjectIdsToRecordId(
        subOptions.where,
        relatedTable
      ) as Record<string, unknown>;
      const conditions = Object.entries(parsedSubWhere).map(([key, value]) => {
        if (value instanceof RecordId) {
          return `${key} = ${value.toString()}`;
        }
        return `${key} = ${JSON.stringify(value)}`;
      });
      subqueryWhere = ` AND ${conditions.join(" AND ")}`;
    }

    // Build sub-orderBy
    if (subOptions.orderBy && Object.keys(subOptions.orderBy).length > 0) {
      const orderClauses = Object.entries(subOptions.orderBy).map(
        ([field, direction]) => `${field} ${direction}`
      );
      subqueryOrderBy = ` ORDER BY ${orderClauses.join(", ")}`;
    }

    // Build sub-limit
    if (subOptions.limit !== undefined) {
      subqueryLimit = ` LIMIT ${subOptions.limit}`;
    }

    // Handle nested relationships
    if (subOptions.related && subOptions.related.length > 0) {
      // Resolve nested relationship metadata if schema is available
      const resolvedNestedRels = subOptions.related.map((nestedRel) => {
        if (schema) {
          // Look up the actual relationship metadata from schema
          const relationship = schema.relationships.find(
            (r) => r.from === relatedTable && r.field === nestedRel.alias
          );

          if (relationship) {
            // Use the resolved table name and add foreign key field
            const nestedForeignKeyField =
              relationship.cardinality === "many"
                ? relatedTable
                : nestedRel.alias;

            return {
              ...nestedRel,
              relatedTable: relationship.to,
              cardinality: relationship.cardinality,
              foreignKeyField: nestedForeignKeyField,
            } as RelatedQuery & { foreignKeyField: string };
          }
        }
        return nestedRel;
      });

      const nestedSubqueries = resolvedNestedRels.map((nestedRel) =>
        buildSubquery(nestedRel, schema)
      );
      subquerySelect += ", " + nestedSubqueries.join(", ");
    }
  }

  // Determine the WHERE condition based on cardinality
  let whereCondition: string;
  if (cardinality === "one") {
    // For one-to-one, the related table's id matches parent's foreign key field
    whereCondition = `WHERE id=$parent.${foreignKeyField}`;
    // Add LIMIT 1 for one-to-one relationships if not already set
    if (!subqueryLimit) {
      subqueryLimit = " LIMIT 1";
    }
  } else {
    // For one-to-many, the related table has a foreign key field pointing to parent's id
    whereCondition = `WHERE ${foreignKeyField}=$parent.id`;
  }

  // Build the complete subquery
  let subquery = `(SELECT ${subquerySelect} FROM ${relatedTable} ${whereCondition}${subqueryWhere}${subqueryOrderBy}${subqueryLimit})`;

  // For one-to-one relationships, select the first element
  if (cardinality === "one") {
    subquery += "[0]";
  }

  subquery += ` AS ${alias}`;

  return subquery;
}
