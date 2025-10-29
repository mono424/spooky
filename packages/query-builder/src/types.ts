// Model types
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
  vars?: Record<string, unknown>;
}

export interface RelatedQuery {
  /** The name of the related table to query */
  relatedTable: string;
  /** The alias for this subquery result (defaults to relatedTable name) */
  alias?: string;
  /** Optional query modifier for the subquery */
  modifier?: QueryModifier<any>;
}

export interface QueryOptions<SModel extends GenericModel> {
  select?: ((keyof SModel & string) | "*")[];
  where?: Partial<SModel>;
  limit?: number;
  offset?: number;
  orderBy?: Partial<Record<keyof SModel, "asc" | "desc">>;
  /** Related tables to include via subqueries */
  related?: RelatedQuery[];
}

export interface LiveQueryOptions<SModel extends GenericModel>
  extends Omit<QueryOptions<SModel>, "orderBy"> {}

// Query modifier type for related queries
export type QueryModifier<SModel extends GenericModel> = (
  builder: QueryModifierBuilder<SModel>
) => QueryModifierBuilder<SModel>;

// Simplified query builder interface for modifying subqueries
export interface QueryModifierBuilder<SModel extends GenericModel> {
  where(conditions: Partial<SModel>): this;
  select(...fields: ((keyof SModel & string) | "*")[]): this;
  limit(count: number): this;
  offset(count: number): this;
  orderBy(field: keyof SModel & string, direction?: "asc" | "desc"): this;
  related<Field extends string>(
    relatedField: Field,
    modifier?: QueryModifier<any>
  ): this;
  _getOptions(): QueryOptions<SModel>;
}

/**
 * Extract fields from a model that are relationship fields (string or string[])
 * Excludes common non-relationship fields like id, created_at, updated_at, etc.
 */
export type RelationshipFields<SModel extends GenericModel> = {
  [K in keyof SModel]: K extends
    | "id"
    | "created_at"
    | "updated_at"
    | "deleted_at"
    ? never
    : SModel[K] extends string | string[] | null | undefined
    ? K
    : never;
}[keyof SModel];

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
  SModel extends Record<string, any>,
  TableName extends string,
  FieldName extends string,
  Relationships
> = FieldName extends keyof SModel
  ? Omit<SModel, FieldName> & {
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
  : SModel;

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
