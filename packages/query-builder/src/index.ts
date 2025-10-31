// Core exports
export {
  QueryBuilder,
  buildQueryFromOptions,
  FinalQuery,
  InnerQuery,
} from "./query-builder";

// Type exports
export type {
  GenericModel,
  GenericSchema,
  RelatedField,
  QueryInfo,
  QueryOptions,
  LiveQueryOptions,
  QueryModifier,
  QueryModifierBuilder,
  RelatedQuery,
  RelationshipDefinition,
  RelationshipsMetadata,
  RelationshipFields,
  InferRelatedModelFromMetadata,
  GetCardinality,
  WithRelated,
  GetRelationshipFields,
  // Old schema metadata types (kept for compatibility)
  ValueType,
  ColumnSchema,
  TableSchemaMetadata,
  Cardinality,
  RelationshipMetadata,
  SchemaMetadataStructure,
} from "./types";

// New array-based schema type helpers
export type {
  SchemaStructure,
  TableNames,
  GetTable,
  TableModel,
  TableRelationships,
  RelationshipFields as RelationshipFieldsFromSchema,
  GetRelationship,
  SchemaToIndexed,
} from "./table-schema";

// Re-export RecordId from surrealdb for convenience
export { RecordId } from "surrealdb";
