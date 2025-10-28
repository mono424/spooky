// Core exports
export { QueryBuilder, createQueryBuilder, buildQueryFromOptions } from "./query-builder";

// Type exports
export type {
  GenericModel,
  GenericSchema,
  Model,
  ModelPayload,
  QueryInfo,
  QueryOptions,
  LiveQueryOptions,
  QueryModifier,
  QueryModifierBuilder,
  RelatedQuery,
  Relationship,
  RelationshipsMetadata,
  RelationshipFields,
  IsArrayRelation,
  InferRelatedModelFromMetadata,
  GetCardinality,
  WithRelated,
  GetRelationshipFields,
} from "./types";

// Re-export RecordId from surrealdb for convenience
export { RecordId } from "surrealdb";
