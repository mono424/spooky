// Import new schema types
export type {
  ValueType,
  ColumnSchema,
  TableSchemaMetadata,
  Cardinality,
  RelationshipMetadata,
  SchemaMetadataStructure,
} from "./table-schema";

// Model types (backward compatibility)
export type GenericModel = Record<string, any>;
export type GenericSchema = Record<string, GenericModel>;

/**
 * Helper to constrain related field names based on relationships metadata
 */
export type RelatedField<T extends string, R> = GetRelationshipFields<T, R> &
  string;

// Query interfaces
export interface QueryInfo {
  query: string;
  hash: number;
  vars?: Record<string, unknown>;
}

export interface RelatedQuery {
  /** The name of the related table to query */
  relatedTable: string;
  /** The alias for this subquery result (defaults to relatedTable name) */
  alias?: string;
  /** Optional query modifier for the subquery */
  modifier?: QueryModifier<any>;
  /** The cardinality of the relationship */
  cardinality: "one" | "many";
}

export interface QueryOptions<TModel extends GenericModel> {
  select?: ((keyof TModel & string) | "*")[];
  where?: Partial<TModel>;
  limit?: number;
  offset?: number;
  orderBy?: Partial<Record<keyof TModel, "asc" | "desc">>;
  /** Related tables to include via subqueries */
  related?: RelatedQuery[];
}

export interface LiveQueryOptions<TModel extends GenericModel>
  extends Omit<QueryOptions<TModel>, "orderBy"> {}

// Query modifier type for related queries
export type QueryModifier<TModel extends GenericModel> = (
  builder: QueryModifierBuilder<TModel>
) => QueryModifierBuilder<TModel>;

// Simplified query builder interface for modifying subqueries
export interface QueryModifierBuilder<TModel extends GenericModel> {
  where(conditions: Partial<TModel>): this;
  select(...fields: ((keyof TModel & string) | "*")[]): this;
  limit(count: number): this;
  offset(count: number): this;
  orderBy(field: keyof TModel & string, direction?: "asc" | "desc"): this;
  related<Field extends string>(
    relatedField: Field,
    modifier?: QueryModifier<any>
  ): this;
  _getOptions(): QueryOptions<TModel>;
}

/**
 * Extract fields from a model that are relationship fields (string or string[])
 * Excludes common non-relationship fields like id, created_at, updated_at, etc.
 */
export type RelationshipFields<TModel extends GenericModel> = {
  [K in keyof TModel]: K extends
    | "id"
    | "created_at"
    | "updated_at"
    | "deleted_at"
    ? never
    : TModel[K] extends string | string[] | null | undefined
    ? K
    : never;
}[keyof TModel];

/**
 * Helper type to infer the related model type from a field name using Relationships metadata
 * Simplified to directly access the nested structure
 */
export type InferRelatedModelFromMetadata<
  Schema extends GenericSchema,
  TableName extends string,
  FieldName extends string,
  Relationships
> = Relationships extends Record<string, Record<string, RelationshipDefinition>>
  ? TableName extends keyof Relationships
    ? FieldName extends keyof Relationships[TableName]
      ? Relationships[TableName][FieldName]["model"]
      : any
    : any
  : any;

/**
 * Get cardinality for a relationship field from metadata
 * Simplified to directly access the nested structure
 */
export type GetCardinality<
  TableName extends string,
  FieldName extends string,
  Relationships
> = Relationships extends Record<string, Record<string, RelationshipDefinition>>
  ? TableName extends keyof Relationships
    ? FieldName extends keyof Relationships[TableName]
      ? Relationships[TableName][FieldName]["cardinality"]
      : "many"
    : "many"
  : "many";

/**
 * Type that transforms a Model by replacing a field with its related records
 * Uses Relationships metadata to determine cardinality and related table
 */
export type WithRelated<
  Schema extends GenericSchema,
  TModel extends Record<string, any>,
  TableName extends string,
  FieldName extends string,
  Relationships
> = FieldName extends keyof TModel
  ? Omit<TModel, FieldName> & {
      [K in FieldName]: GetCardinality<
        TableName,
        FieldName,
        Relationships
      > extends "one"
        ? InferRelatedModelFromMetadata<
            Schema,
            TableName,
            K,
            Relationships
          > | null
        :
            | InferRelatedModelFromMetadata<
                Schema,
                TableName,
                K,
                Relationships
              >[]
            | null;
    }
  : TModel;

/**
 * Type to extract relationship fields from Relationships metadata
 * Now simplified to just get the keys of the nested object
 */
export type GetRelationshipFields<
  TableName extends string,
  Relationships
> = Relationships extends Record<string, Record<string, any>>
  ? TableName extends keyof Relationships
    ? keyof Relationships[TableName] & string
    : never
  : never;

/**
 * Relationship metadata structure - now a nested object for better type safety
 * Example:
 * {
 *   thread: {
 *     author: { model: Schema["user"], table: "user", cardinality: "one" },
 *     comments: { model: Schema["comment"], table: "comment", cardinality: "many" }
 *   }
 * }
 */
export interface RelationshipDefinition<Model = any> {
  /** The related model type */
  model: Model;
  /** The related table name */
  table: string;
  /** Whether this is a 1:1 or 1:many relationship */
  cardinality: "one" | "many";
}

export type RelationshipsMetadata = Record<
  string,
  Record<string, RelationshipDefinition>
>;
