import { RecordId } from "surrealdb";
import type {
  GenericModel,
  QueryInfo,
  QueryOptions,
  QueryModifier,
  QueryModifierBuilder,
  RelatedQuery,
} from "./types";
import type {
  TableNames,
  GetTable,
  TableModel,
  TableRelationships,
  GetRelationship,
  SchemaStructure,
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

/**
 * Internal query modifier builder implementation
 */
class QueryModifierBuilderImpl<TModel extends GenericModel>
  implements QueryModifierBuilder<TModel>
{
  private options: QueryOptions<TModel> = {};

  where(conditions: Partial<TModel>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  select(...fields: ((keyof TModel & string) | "*")[]): this {
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
    field: keyof TModel & string,
    direction: "asc" | "desc" = "asc"
  ): this {
    this.options.orderBy = {
      ...this.options.orderBy,
      [field]: direction,
    } as Partial<Record<keyof TModel, "asc" | "desc">>;
    return this;
  }

  // Broad implementation to satisfy QueryModifierBuilder interface
  related<Field extends string>(
    relatedField: Field,
    modifier?: QueryModifier<GenericModel>
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
        cardinality: "many", // Default to many for subqueries without explicit cardinality
      });
    }
    return this;
  }

  _getOptions(): QueryOptions<TModel> {
    return this.options;
  }
}

/**
 * Fluent query builder for constructing queries with chainable methods
 * Now with full type inference from schema constant!
 */
export class QueryBuilder<
  const S extends SchemaStructure,
  TableName extends TableNames<S>
> {
  private options: QueryOptions<TableModel<GetTable<S, TableName>>> = {};

  constructor(
    private readonly schema: S,
    private readonly tableName: TableName
  ) {}

  /**
   * Add additional where conditions
   */
  where(conditions: Partial<TableModel<GetTable<S, TableName>>>): this {
    this.options.where = { ...this.options.where, ...conditions };
    return this;
  }

  /**
   * Specify fields to select
   */
  select(
    ...fields: ((keyof TableModel<GetTable<S, TableName>> & string) | "*")[]
  ): this {
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
    field: keyof TableModel<GetTable<S, TableName>>,
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

  /**
   * Add limit to the query (only for non-live queries)
   */
  limit(count: number): this {
    this.options.limit = count;
    return this;
  }

  /**
   * Add offset to the query (only for non-live queries)
   */
  offset(count: number): this {
    this.options.offset = count;
    return this;
  }

  /**
   * Include related data via subqueries
   * Field and cardinality are validated against schema relationships
   */
  related<
    Field extends TableRelationships<S, TableName>["field"],
    Rel extends GetRelationship<S, TableName, Field>
  >(
    field: Field,
    cardinalityOrModifier?: Rel["cardinality"] | QueryModifier<GenericModel>,
    modifier?: QueryModifier<GenericModel>
  ): this {
    if (!this.options.related) {
      this.options.related = [];
    }

    // Determine if first optional arg is cardinality or modifier
    const actualModifier =
      typeof cardinalityOrModifier === "function"
        ? cardinalityOrModifier
        : modifier;

    const relationship = this.schema.relationships.find(
      (r) => r.from === this.tableName && r.field === field
    );

    if (!relationship) {
      throw new Error(
        `Relationship '${field}' not found for table '${this.tableName}'`
      );
    }

    const exists = this.options.related.some(
      (r) => (r.alias || r.relatedTable) === field
    );

    if (!exists) {
      const foreignKeyField =
        relationship.cardinality === "many" ? this.tableName : field;

      this.options.related.push({
        relatedTable: relationship.to,
        alias: field,
        modifier: actualModifier,
        cardinality: relationship.cardinality,
        foreignKeyField,
      } as RelatedQuery & { foreignKeyField: string });
    }

    return this;
  }

  /**
   * Get the current query options
   */
  getOptions(): QueryOptions<TableModel<GetTable<S, TableName>>> {
    return this.options;
  }

  /**
   * Build a SQL query string from the current builder state
   * @param method - The query method (SELECT or LIVE SELECT)
   * @returns QueryInfo with the generated SQL and variables
   */
  buildQuery(method: "LIVE SELECT" | "SELECT" = "SELECT"): QueryInfo {
    return buildQueryFromOptions(method, this.tableName, this.options);
  }

  /**
   * Build a live query (LIVE SELECT)
   */
  buildLiveQuery(): QueryInfo {
    return this.buildQuery("LIVE SELECT");
  }
}

/**
 * Build a query string from query options
 * @param method - The query method (SELECT or LIVE SELECT)
 * @param tableName - The table name to query
 * @param options - The query options (where, select, orderBy, etc.)
 * @returns QueryInfo with the generated SQL and variables
 */
export function buildQueryFromOptions<TModel extends GenericModel>(
  method: "SELECT" | "LIVE SELECT",
  tableName: string,
  options: QueryOptions<TModel>
): QueryInfo {
  const isLiveQuery = method === "LIVE SELECT";

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
  if (options.related && options.related.length > 0) {
    const subqueries = options.related.map((rel) => buildSubquery(rel));
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
    vars: Object.keys(vars).length > 0 ? vars : undefined,
  };
}

/**
 * Build a subquery for a related field
 */
function buildSubquery(
  rel: RelatedQuery & { foreignKeyField?: string }
): string {
  const { relatedTable, alias, modifier, cardinality } = rel;
  const foreignKeyField = rel.foreignKeyField || alias;

  let subquerySelect = "*";
  let subqueryWhere = "";
  let subqueryOrderBy = "";
  let subqueryLimit = "";

  // If there's a modifier, apply it to get the sub-options
  if (modifier) {
    const modifierBuilder = new QueryModifierBuilderImpl();
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
      const nestedSubqueries = subOptions.related.map((nestedRel) =>
        buildSubquery(nestedRel)
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
