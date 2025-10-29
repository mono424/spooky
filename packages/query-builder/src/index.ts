// Core exports
export { QueryBuilder, createQueryBuilder, buildQueryFromOptions } from "./query-builder";

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
} from "./types";

// Re-export RecordId from surrealdb for convenience
export { RecordId } from "surrealdb";
