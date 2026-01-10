// Core exports
export {
  type Executor,
  QueryBuilder,
  buildQueryFromOptions,
  FinalQuery,
  InnerQuery,
  type QueryResult,
  type RelatedFieldsMap,
  type BuildRelatedFields,
  type BuildResultModelOne,
  type BuildResultModelMany,
  type ExtractFieldNames,
  type RelatedFieldMapEntry,
  cyrb53,
} from './query-builder';

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
  SchemaAwareQueryModifier,
  SchemaAwareQueryModifierBuilder,
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
} from './types';

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
  AccessDefinition,
  TypeNameToTypeMap,
} from './table-schema';

// Re-export RecordId from surrealdb for convenience
export { RecordId } from 'surrealdb';
