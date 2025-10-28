import { Doc, RecordId } from "surrealdb";

// Model types
export type GenericModel = Record<string, any>;
export type GenericSchema = Record<string, GenericModel>;
// ModelPayload keeps the types from the schema (string IDs)
export type ModelPayload<T> = T;
export type Model<T> = ModelPayload<T>;

export type RelatedField<T extends string, R> = GetRelationshipFields<T, R> &
  string;

// Internal type used for database operations (with RecordId)
export type InternalModel<T> = Omit<T, "id"> & { id: RecordId };

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
  where?: Partial<Model<SModel>>;
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
  where(conditions: Partial<Model<SModel>>): this;
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
    : SModel[K] extends
        | string
        | string[]
        | (string[] | null)
        | (string | null)
        | undefined
    ? K
    : never;
}[keyof SModel];

/**
 * Check if a field is a 1:many relationship (array type)
 */
export type IsArrayRelation<
  SModel extends GenericModel,
  Field extends keyof SModel
> = SModel[Field] extends
  | string[]
  | (string[] | null)
  | (string[] | null | undefined)
  ? true
  : false;

/**
 * Helper type to infer the related model type from a field name using Relationships metadata
 * Falls back to pluralization heuristic if metadata not available
 */
export type InferRelatedModelFromMetadata<
  Schema extends GenericSchema,
  TableName extends string,
  FieldName extends string,
  Relationships
> = Relationships extends Record<
  string,
  Array<{ field: string; table: string }>
>
  ? TableName extends keyof Relationships
    ? Extract<Relationships[TableName][number], { field: FieldName }> extends {
        table: infer RelatedTable;
      }
      ? RelatedTable extends keyof Schema
        ? // Handle junction tables - map them to the actual related table
          RelatedTable extends "commented_on"
          ? Schema["comment"]
          : Schema[RelatedTable]
        : any
      : any
    : any
  : // Fallback to pluralization if no metadata
  FieldName extends `${infer Singular}s`
  ? Singular extends keyof Schema
    ? Schema[Singular]
    : any
  : FieldName extends keyof Schema
  ? Schema[FieldName]
  : any;

/**
 * Get cardinality for a relationship field from metadata
 */
export type GetCardinality<
  TableName extends string,
  FieldName extends string,
  Relationships
> = Relationships extends Record<
  string,
  Array<{ field: string; cardinality?: string }>
>
  ? TableName extends keyof Relationships
    ? Extract<Relationships[TableName][number], { field: FieldName }> extends {
        cardinality: infer C;
      }
      ? C
      : "many" // Default to many if not specified
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
  ? GetCardinality<TableName, FieldName, Relationships> extends "one"
    ? Omit<SModel, FieldName> & {
        [K in FieldName]: InferRelatedModelFromMetadata<
          Schema,
          TableName,
          K,
          Relationships
        > | null;
      }
    : Omit<SModel, FieldName> & {
        [K in FieldName]:
          | InferRelatedModelFromMetadata<Schema, TableName, K, Relationships>[]
          | null;
      }
  : SModel;

/**
 * Type to extract relationship fields from Relationships metadata
 * Only returns actual relationship fields, not all string fields
 */
export type GetRelationshipFields<
  TableName extends string,
  Relationships
> = Relationships extends Record<string, Array<{ field: string }>>
  ? TableName extends keyof Relationships
    ? Relationships[TableName][number]["field"]
    : never
  : never;

/**
 * Relationship metadata structure
 */
export interface Relationship {
  /** The field name that creates this relationship */
  field: string;
  /** The related table name */
  table: string;
  /** Whether this is a 1:1 or 1:many relationship */
  cardinality?: "one" | "many";
}

export type RelationshipsMetadata = Record<string, Relationship[]>;
