/**
 * Supported value types in the schema
 */
export type ValueType = "string" | "number" | "boolean" | "null" | "json";

/**
 * Column metadata defining the type and optionality of a field
 */
export interface ColumnSchema {
  readonly type: ValueType;
  readonly optional: boolean;
}

/**
 * Table metadata containing columns and primary key information
 */
export interface TableSchemaMetadata {
  readonly name: string;
  readonly columns: {
    readonly [columnName: string]: ColumnSchema;
  };
  readonly primaryKey: readonly string[];
}

/**
 * Cardinality of a relationship: one-to-one or one-to-many
 */
export type Cardinality = "one" | "many";

/**
 * Relationship metadata defining how tables relate to each other
 */
export interface RelationshipMetadata {
  readonly model: string;
  readonly table: string;
  readonly cardinality: Cardinality;
}

/**
 * Complete schema metadata structure
 * Maps table names to their schemas and relationships
 */
export interface SchemaMetadataStructure {
  readonly tables: {
    readonly [tableName: string]: TableSchemaMetadata;
  };
  readonly relationships: {
    readonly [tableName: string]: {
      readonly [fieldName: string]: RelationshipMetadata;
    };
  };
}

/**
 * Type mapping from ValueType to TypeScript types
 */
export type TypeNameToTypeMap = {
  string: string;
  number: number;
  boolean: boolean;
  null: null;
  json: unknown;
};

/**
 * Convert a column type to its TypeScript type
 */
export type ColumnToTSType<T extends ColumnSchema> = T extends {
  optional: true;
}
  ? TypeNameToTypeMap[T["type"]] | null
  : TypeNameToTypeMap[T["type"]];

/**
 * Helper to extract relationship field names for a table
 */
export type RelationshipFieldNames<
  Metadata extends SchemaMetadataStructure,
  TableName extends keyof Metadata["relationships"] & string
> = keyof Metadata["relationships"][TableName] & string;

/**
 * Helper to get cardinality for a specific relationship
 */
export type GetRelationshipCardinality<
  Metadata extends SchemaMetadataStructure,
  TableName extends keyof Metadata["relationships"] & string,
  FieldName extends keyof Metadata["relationships"][TableName] & string
> = Metadata["relationships"][TableName][FieldName]["cardinality"];

/**
 * Helper to get related table name for a relationship
 */
export type GetRelatedTable<
  Metadata extends SchemaMetadataStructure,
  TableName extends keyof Metadata["relationships"] & string,
  FieldName extends keyof Metadata["relationships"][TableName] & string
> = Metadata["relationships"][TableName][FieldName]["table"];

// ============================================================================
// ARRAY-BASED SCHEMA TYPE HELPERS
// ============================================================================

/**
 * Base schema structure for array-based schemas
 */
export interface SchemaStructure {
  readonly tables: readonly {
    readonly name: string;
    readonly columns: Record<string, ColumnSchema>;
    readonly primaryKey: readonly string[];
  }[];
  readonly relationships: readonly {
    readonly from: string;
    readonly field: string;
    readonly to: string;
    readonly cardinality: Cardinality;
  }[];
}

/**
 * Extract a specific table by name from the schema tables array
 */
export type GetTable<
  S extends SchemaStructure,
  Name extends string
> = Extract<S["tables"][number], { name: Name }>;

/**
 * Extract all table names from the schema
 */
export type TableNames<S extends SchemaStructure> =
  S["tables"][number]["name"];

/**
 * Convert table schema columns to a TypeScript model type
 */
export type TableModel<T extends { columns: Record<string, ColumnSchema> }> = {
  [K in keyof T["columns"]]: T["columns"][K] extends { optional: true }
    ? TypeNameToTypeMap[T["columns"][K]["type"]] | null
    : TypeNameToTypeMap[T["columns"][K]["type"]];
};

/**
 * Extract all relationships for a specific table from relationships array
 */
export type TableRelationships<
  S extends SchemaStructure,
  TableName extends string
> = Extract<S["relationships"][number], { from: TableName }>;

/**
 * Get relationship field names for a table
 */
export type RelationshipFields<
  S extends SchemaStructure,
  TableName extends string
> = TableRelationships<S, TableName>["field"];

/**
 * Get specific relationship by table and field
 */
export type GetRelationship<
  S extends SchemaStructure,
  TableName extends string,
  Field extends string
> = Extract<
  Extract<S["relationships"][number], { from: TableName }>,
  { field: Field }
>;

/**
 * Convert array-based schema to indexed format (for internal compatibility)
 */
export type SchemaToIndexed<S extends SchemaStructure> = {
  tables: {
    [K in S["tables"][number]["name"]]: Extract<
      S["tables"][number],
      { name: K }
    >;
  };
  relationships: {
    [K in S["tables"][number]["name"]]: {
      [R in Extract<S["relationships"][number], { from: K }>["field"]]: Extract<
        Extract<S["relationships"][number], { from: K }>,
        { field: R }
      >;
    };
  };
};
