/// Supported value types in the schema
enum ValueType {
  string,
  number,
  boolean,
  nullValue, // null is a reserved keyword
  json,
}

/// Column metadata defining the type and optionality of a field
class ColumnSchema {
  final ValueType type;
  final bool optional;
  final bool? dateTime;
  final bool? recordId;

  const ColumnSchema({
    required this.type,
    required this.optional,
    this.dateTime,
    this.recordId,
  });
}

/// Table metadata containing columns and primary key information
class TableSchemaMetadata {
  final String name;
  final Map<String, ColumnSchema> columns;
  final List<String> primaryKey;

  const TableSchemaMetadata({
    required this.name,
    required this.columns,
    required this.primaryKey,
  });
}

/// Cardinality of a relationship: one-to-one or one-to-many
enum Cardinality { one, many }

/// Relationship metadata defining how tables relate to each other
class RelationshipMetadata {
  final String model;
  final String table;
  final Cardinality cardinality;

  const RelationshipMetadata({
    required this.model,
    required this.table,
    required this.cardinality,
  });
}

/// Complete schema metadata structure
/// Maps table names to their schemas and relationships
class SchemaMetadataStructure {
  final Map<String, TableSchemaMetadata> tables;
  final Map<String, Map<String, RelationshipMetadata>> relationships;

  const SchemaMetadataStructure({
    required this.tables,
    required this.relationships,
  });
}

// ============================================================================
// ARRAY-BASED SCHEMA TYPE HELPERS (Ported as Classes)
// ============================================================================

class TableDefinition {
  final String name;
  final Map<String, ColumnSchema> columns;
  final List<String> primaryKey;

  const TableDefinition({
    required this.name,
    required this.columns,
    required this.primaryKey,
  });
}

class RelationshipDefinition {
  final String from;
  final String field;
  final String to;
  final Cardinality cardinality;

  const RelationshipDefinition({
    required this.from,
    required this.field,
    required this.to,
    required this.cardinality,
  });
}

class SchemaStructure {
  final List<TableDefinition> tables;
  final List<RelationshipDefinition> relationships;

  const SchemaStructure({required this.tables, required this.relationships});
}
