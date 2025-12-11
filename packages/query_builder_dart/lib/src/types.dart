/// Supported value types in the schema
enum ValueType { string, number, boolean, nullValue, json, datetime, record }

/// Cardinality of a relationship
enum Cardinality { one, many }

/// Column metadata
class ColumnSchema {
  final ValueType type;
  final bool optional;
  final bool dateTime;
  final bool recordId;

  const ColumnSchema({
    required this.type,
    this.optional = false,
    this.dateTime = false,
    this.recordId = false,
  });
}

/// Table metadata
class TableSchema {
  final String name;
  final Map<String, ColumnSchema> columns;
  final List<String> primaryKey;

  const TableSchema({
    required this.name,
    required this.columns,
    this.primaryKey = const [],
  });
}

/// Relationship metadata
class Relationship {
  final String from;
  final String field;
  final String to;
  final Cardinality cardinality;

  const Relationship({
    required this.from,
    required this.field,
    required this.to,
    required this.cardinality,
  });
}

/// Complete schema structure
class SchemaStructure {
  final List<TableSchema> tables;
  final List<Relationship> relationships;

  const SchemaStructure({required this.tables, required this.relationships});

  TableSchema? getTable(String name) {
    try {
      return tables.firstWhere((t) => t.name == name);
    } catch (_) {
      return null;
    }
  }

  Relationship? getRelationship(String from, String field) {
    try {
      return relationships.firstWhere(
        (r) => r.from == from && r.field == field,
      );
    } catch (_) {
      return null;
    }
  }
}

/// Query options
class QueryOptions {
  List<String>? select;
  Map<String, dynamic>? where;
  int? limit;
  int? offset;
  Map<String, String>? orderBy; // field -> 'asc' | 'desc'
  List<RelatedQuery>? related;
  bool isOne;

  QueryOptions({
    this.select,
    this.where,
    this.limit,
    this.offset,
    this.orderBy,
    this.related,
    this.isOne = false,
  });

  QueryOptions copyWith({
    List<String>? select,
    Map<String, dynamic>? where,
    int? limit,
    int? offset,
    Map<String, String>? orderBy,
    List<RelatedQuery>? related,
    bool? isOne,
  }) {
    return QueryOptions(
      select: select ?? this.select,
      where: where ?? this.where,
      limit: limit ?? this.limit,
      offset: offset ?? this.offset,
      orderBy: orderBy ?? this.orderBy,
      related: related ?? this.related,
      isOne: isOne ?? this.isOne,
    );
  }
}

/// Related query definition
class RelatedQuery {
  final String relatedTable;
  final String alias;
  final Function? modifier; // Function that takes a builder
  final Cardinality cardinality;
  final String foreignKeyField;

  RelatedQuery({
    required this.relatedTable,
    required this.alias,
    this.modifier,
    required this.cardinality,
    required this.foreignKeyField,
  });
}

/// Result of a query build
class QueryInfo {
  final String query;
  final Map<String, dynamic> vars;

  QueryInfo(this.query, [this.vars = const {}]);
}
