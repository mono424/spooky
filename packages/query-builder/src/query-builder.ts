import { RecordId } from "surrealdb";
import type {
  GenericModel,
  GenericSchema,
  Model,
  QueryInfo,
  QueryOptions,
  LiveQueryOptions,
  QueryModifier,
  QueryModifierBuilder,
  RelatedQuery,
  RelationshipsMetadata,
  WithRelated,
  GetRelationshipFields,
  RelatedField,
  InferRelatedModelFromMetadata,
} from "./types";

/**
 * Parse a string ID to RecordId
 * - If it's in the format "table:id", use it as-is
 * - If it's just an ID without ":", prepend the table name
 * @param value - The value to parse (could be a string ID)
 * @param tableName - The table name to use if the ID doesn't contain ":"
 * @param fieldName - The field name to determine if this is an ID field
 */
function parseStringToRecordId(
  value: any,
  tableName?: string,
  fieldName?: string
): any {
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
function parseObjectIdsToRecordId(obj: any, tableName?: string): any {
  if (obj === null || obj === undefined) return obj;

  if (typeof obj === "string") {
    return parseStringToRecordId(obj, tableName);
  }

  if (Array.isArray(obj)) {
    return obj.map((item) => parseObjectIdsToRecordId(item, tableName));
  }

  if (typeof obj === "object" && obj.constructor === Object) {
    const result: any = {};
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

/**
 * Internal query modifier builder implementation
 */
class QueryModifierBuilderImpl<
  TableName extends string,
  SModel extends GenericModel,
  Relationships extends Record<
    string,
    Array<{ field: string; table: string; cardinality?: "one" | "many" }>
  >
> implements QueryModifierBuilder<SModel>
{
  private options: QueryOptions<SModel> = {};

  where(conditions: Partial<Model<SModel>>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  select(...fields: ((keyof SModel & string) | "*")[]): this {
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
    field: keyof SModel & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<Record<keyof SModel, "asc" | "desc">>;
    return this;
  }

  // Broad implementation to satisfy QueryModifierBuilder interface
  related<Field extends string>(
    relatedField: Field,
    modifier?: QueryModifier<any>
  ): this {
    if (!this.options.related) {
      this.options.related = [];
    }

    const exists = this.options.related.some(
      (r) => (r.alias || r.relatedTable) === relatedField
    );

    if (!exists) {
      this.options.related.push({
        relatedTable: relatedField,
        alias: relatedField,
        modifier,
      });
    }
    return this;
  }

  _getOptions(): QueryOptions<SModel> {
    return this.options;
  }
}

/**
 * Fluent query builder for constructing queries with chainable methods
 */
export class QueryBuilder<
  Schema extends GenericSchema,
  SModel extends Record<string, any>,
  TableName extends keyof Schema & string = keyof Schema & string,
  Relationships = undefined
> {
  private options: QueryOptions<SModel> = {};
  private relatedFields: string[] = [];

  constructor(
    private currentTableName: TableName,
    private relationships?: RelationshipsMetadata,
    where?: Partial<Model<SModel>>
  ) {
    if (where) {
      this.options.where = where;
    }
  }

  /**
   * Add additional where conditions
   */
  where(conditions: Partial<Model<SModel>>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  /**
   * Specify fields to select
   */
  select(...fields: ((keyof SModel & string) | "*")[]): this {
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
    field: keyof SModel & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<Record<keyof SModel, "asc" | "desc">>;
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
   * Include related records from specified table(s) using subqueries
   * @param relatedField - The relationship field name to expand (e.g., "comments")
   * @param modifier - Optional function to modify the subquery (e.g., add where, limit)
   * @example
   * // For a thread table that has relationship to comments
   * queryBuilder.related("comments")
   * // With modifier to filter and limit comments
   * queryBuilder.related("comments", (q) => q.where({ approved: true }).limit(5))
   * // Nested related queries
   * queryBuilder.related("comments", (q) => q.related("author").limit(10))
   */
  related<RF extends RelatedField<TableName, Relationships>>(
    relatedField: RF,
    modifier?: QueryModifier<
      InferRelatedModelFromMetadata<
        Schema,
        TableName,
        RF & string,
        Relationships
      >
    >
  ): QueryBuilder<
    Schema,
    WithRelated<Schema, SModel, TableName, RF & string, Relationships>,
    TableName,
    Relationships
  > {
    if (!this.options.related) {
      this.options.related = [];
    }

    // Check if this relationship already exists
    const exists = this.options.related.some(
      (r) => (r.alias || r.relatedTable) === relatedField
    );

    if (!exists) {
      this.options.related.push({
        relatedTable: relatedField,
        alias: relatedField,
        modifier,
      });
      this.relatedFields.push(relatedField);
    }
    return this as any;
  }

  /**
   * Get the built query options
   */
  getOptions(): QueryOptions<SModel> {
    return this.options;
  }

  /**
   * Build a SQL query string from the current builder state
   * @param method - The query method (SELECT or LIVE SELECT)
   * @returns QueryInfo with the generated SQL and variables
   */
  buildQuery(method: "LIVE SELECT" | "SELECT" = "SELECT"): QueryInfo {
    return buildQueryFromOptions(
      method,
      this.currentTableName,
      this.options,
      this.relationships
    );
  }

  /**
   * Build a live query (LIVE SELECT)
   */
  buildLiveQuery(): QueryInfo {
    return this.buildQuery("LIVE SELECT");
  }
}

/**
 * Generate a subquery for related records using relationship metadata
 * @param currentTable - The current table being queried
 * @param relatedAlias - The alias/name for the related data (e.g., "comments")
 * @param relationships - Relationship metadata
 * @param modifier - Optional query modifier function
 * @returns SQL subquery string
 */
function generateSubquery(
  currentTable: string,
  relatedAlias: string,
  relationships?: RelationshipsMetadata,
  modifier?: QueryModifier<any>
): string {
  // Base query parts
  let selectClause = "*";
  let whereClause = "";
  let orderClause = "";
  let limitClause = "";
  let offsetClause = "";
  const subqueries: string[] = [];

  // Apply modifier if provided
  if (modifier) {
    const modifierBuilder = new QueryModifierBuilderImpl();
    modifier(modifierBuilder);
    const modifierOptions = modifierBuilder._getOptions();

    // Apply select
    if (modifierOptions.select && modifierOptions.select.length > 0) {
      selectClause = modifierOptions.select.join(", ");
    }

    // Build where conditions
    const conditions: string[] = [];

    if (modifierOptions.where) {
      for (const [key, value] of Object.entries(modifierOptions.where)) {
        conditions.push(`${key} = ${formatValue(value)}`);
      }
    }

    // Handle related subqueries recursively
    if (modifierOptions.related && modifierOptions.related.length > 0) {
      // Determine the actual table name for nested queries
      const relatedTable = getRelatedTableName(
        currentTable,
        relatedAlias,
        relationships
      );

      for (const rel of modifierOptions.related) {
        const nestedSubquery = generateSubquery(
          relatedTable,
          rel.alias || rel.relatedTable,
          relationships,
          rel.modifier
        );
        subqueries.push(
          `${nestedSubquery} AS ${rel.alias || rel.relatedTable}`
        );
      }
    }

    if (conditions.length > 0) {
      whereClause = conditions.join(" AND ");
    }

    // Apply order by
    if (modifierOptions.orderBy) {
      const orderParts = Object.entries(modifierOptions.orderBy)
        .map(([key, dir]) => `${key} ${dir}`)
        .join(", ");
      if (orderParts) {
        orderClause = orderParts;
      }
    }

    // Apply limit/offset
    if (modifierOptions.limit !== undefined) {
      limitClause = `LIMIT ${modifierOptions.limit}`;
    }
    if (modifierOptions.offset !== undefined) {
      offsetClause = `START ${modifierOptions.offset}`;
    }
  }

  // Try to find the relationship in metadata
  let baseQuery = "";
  if (relationships && relationships[currentTable]) {
    const relationship = relationships[currentTable].find(
      (rel: any) => rel.field === relatedAlias
    );
    if (relationship) {
      const { table: relatedTable, cardinality } = relationship;

      if (cardinality === "many") {
        // For many-to-many relationships through junction tables
        if (relatedTable === "commented_on") {
          const allSelectParts =
            subqueries.length > 0
              ? [selectClause, ...subqueries].join(", ")
              : selectClause;
          baseQuery = `(SELECT ${allSelectParts} FROM comment WHERE id IN (SELECT in FROM ${relatedTable} WHERE out=$parent.id)`;
        } else {
          const allSelectParts =
            subqueries.length > 0
              ? [selectClause, ...subqueries].join(", ")
              : selectClause;
          baseQuery = `(SELECT ${allSelectParts} FROM ${relatedTable} WHERE ${currentTable}=$parent.id`;
        }
      } else {
        // For one-to-one relationships
        const allSelectParts =
          subqueries.length > 0
            ? [selectClause, ...subqueries].join(", ")
            : selectClause;
        baseQuery = `(SELECT ${allSelectParts} FROM ${relatedTable} WHERE ${currentTable}=$parent.id`;
      }
    }
  }

  // Fallback to simple pluralization logic
  if (!baseQuery) {
    let relatedTable = relatedAlias;
    if (relatedAlias.endsWith("s") && relatedAlias.length > 1) {
      relatedTable = relatedAlias.slice(0, -1);
    }

    const foreignKey = currentTable;
    const allSelectParts =
      subqueries.length > 0
        ? [selectClause, ...subqueries].join(", ")
        : selectClause;
    baseQuery = `(SELECT ${allSelectParts} FROM ${relatedTable} WHERE ${foreignKey}=$parent.id`;
  }

  // Add where clause if any
  if (whereClause) {
    baseQuery += ` AND ${whereClause}`;
  }

  // Add order clause
  if (orderClause) {
    baseQuery += ` ORDER BY ${orderClause}`;
  }

  // Add limit/offset
  if (limitClause) {
    baseQuery += ` ${limitClause}`;
  }
  if (offsetClause) {
    baseQuery += ` ${offsetClause}`;
  }

  baseQuery += ")";

  return baseQuery;
}

/**
 * Get the actual table name for a related field using relationship metadata
 */
function getRelatedTableName(
  currentTable: string,
  relatedAlias: string,
  relationships?: RelationshipsMetadata
): string {
  if (relationships && relationships[currentTable]) {
    const relationship = relationships[currentTable].find(
      (rel: any) => rel.field === relatedAlias
    );
    if (relationship) {
      // Handle junction tables
      if (relationship.table === "commented_on") {
        return "comment";
      }
      return relationship.table;
    }
  }

  // Fallback to simple pluralization
  if (relatedAlias.endsWith("s") && relatedAlias.length > 1) {
    return relatedAlias.slice(0, -1);
  }
  return relatedAlias;
}

/**
 * Format a value for SQL query (handles RecordId, strings, etc.)
 */
function formatValue(value: any): string {
  // Parse string IDs to RecordId first
  const parsedValue = parseStringToRecordId(value);

  if (parsedValue instanceof RecordId) {
    return `${parsedValue}`;
  }
  if (typeof parsedValue === "string") {
    // Don't quote if it looks like it should have been a RecordId
    if (parsedValue.includes(":")) {
      return parsedValue;
    }
    return `"${parsedValue.replace(/"/g, '\\"')}"`;
  }
  if (typeof parsedValue === "number") {
    return String(parsedValue);
  }
  if (typeof parsedValue === "boolean") {
    return parsedValue ? "true" : "false";
  }
  if (parsedValue === null) {
    return "NULL";
  }
  return JSON.stringify(parsedValue);
}

/**
 * Build a query from options
 * @param method - The query method (SELECT or LIVE SELECT)
 * @param tableName - The table name
 * @param options - Query options
 * @param relationships - Relationship metadata
 * @returns QueryInfo with the generated SQL and variables
 */
export function buildQueryFromOptions<SModel extends GenericModel>(
  method: "LIVE SELECT" | "SELECT",
  tableName: string,
  options: QueryOptions<SModel> | LiveQueryOptions<SModel>,
  relationships?: RelationshipsMetadata
): QueryInfo {
  // Build the select clause with subqueries for related data
  let selectParts = options.select ?? ["*"];
  const selectFields = selectParts.map((key) => `${key}`).join(", ");

  // Add subqueries for related tables
  const subqueries: string[] = [];
  if (options.related && options.related.length > 0) {
    for (const rel of options.related) {
      const subquery = generateSubquery(
        tableName,
        rel.alias || rel.relatedTable,
        relationships,
        rel.modifier
      );
      const alias = rel.alias || rel.relatedTable;
      subqueries.push(`${subquery} AS ${alias}`);
    }
  }

  // Combine regular fields and subqueries
  const allSelectParts = [selectFields, ...subqueries].join(", ");

  // Parse where conditions to convert string IDs to RecordId
  // Pass the table name so IDs without ":" get the table prepended automatically
  const parsedWhere = parseObjectIdsToRecordId(options.where ?? {}, tableName);

  const whereClause = Object.keys(parsedWhere)
    .map((key) => `${key} = $${key}`)
    .join(" AND ");

  // Only add ORDER BY for non-live queries
  const orderClause =
    method === "SELECT" && "orderBy" in options
      ? Object.entries((options as QueryOptions<SModel>).orderBy ?? {})
          .map(([key, val]) => `${key} ${val}`)
          .join(", ")
      : "";

  let query = `${method} ${allSelectParts} FROM ${tableName}`;
  if (whereClause) query += ` WHERE ${whereClause}`;
  if (orderClause) query += ` ORDER BY ${orderClause}`;

  // Only add LIMIT and START for regular SELECT queries (not LIVE SELECT)
  if (method === "SELECT") {
    if (options.limit !== undefined) query += ` LIMIT ${options.limit}`;
    if (options.offset !== undefined) query += ` START ${options.offset}`;
  }

  query += ";";

  console.log(`[buildQuery] Generated ${method} query:`, query);
  console.log(`[buildQuery] Query vars:`, parsedWhere);

  return {
    query,
    vars: parsedWhere,
  };
}

/**
 * Create a new query builder instance
 */
export function createQueryBuilder<
  Schema extends GenericSchema,
  SModel extends Record<string, any>,
  TableName extends keyof Schema & string,
  Relationships = undefined
>(
  tableName: TableName,
  relationships?: RelationshipsMetadata,
  where?: Partial<Model<SModel>>
): QueryBuilder<Schema, SModel, TableName, Relationships> {
  return new QueryBuilder<Schema, SModel, TableName, Relationships>(
    tableName,
    relationships,
    where
  );
}
