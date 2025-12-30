import 'table_schema.dart';

class RecordId {
  final String tb;
  final String id;

  RecordId(this.tb, this.id);

  @override
  String toString() {
    // Basic SURQL formatting.
    // Ideally we should handle complex IDs (wrapping in ⟨ ⟩), but for now simple concatenation matches most cases.
    return '$tb:$id';
  }

  // Helper to print debug friendly string if needed, but toString is used in query gen
  String toSurql() => toString();
}

// Model types
typedef GenericModel = Map<String, dynamic>;

class QueryInfo {
  final String query;
  final int hash;
  final Map<String, dynamic>? vars;

  QueryInfo({required this.query, required this.hash, this.vars});

  @override
  String toString() => 'QueryInfo(query: $query, hash: $hash, vars: $vars)';
}

class RelatedQuery {
  /// The name of the related table to query
  final String relatedTable;

  /// The alias for this subquery result (defaults to relatedTable name)
  final String? alias;

  /// Optional query modifier for the subquery
  final QueryModifier? modifier;

  /// The cardinality of the relationship
  final Cardinality cardinality;

  /// Internal: The field to use for foreign key lookup
  final String? foreignKeyField;

  RelatedQuery({
    required this.relatedTable,
    required this.cardinality,
    this.alias,
    this.modifier,
    this.foreignKeyField,
  });
}

class QueryOptions {
  List<String>? select;
  Map<String, dynamic>? where;
  int? limit;
  int? offset;
  Map<String, String>? orderBy; // field -> "asc" | "desc"
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

  // Deep copy helper
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

// Query modifier type
typedef QueryModifier =
    QueryModifierBuilder Function(QueryModifierBuilder builder);

// Simplified query builder interface for modifying subqueries
abstract class QueryModifierBuilder {
  QueryModifierBuilder where(Map<String, dynamic> conditions);
  QueryModifierBuilder select(List<String> fields);
  QueryModifierBuilder limit(int count);
  QueryModifierBuilder offset(int count);
  QueryModifierBuilder orderBy(String field, {String direction = "asc"});
  QueryModifierBuilder related(
    String relatedField, {
    QueryModifier? modifier,
    Cardinality? cardinality,
  });

  QueryOptions getOptions();
}
