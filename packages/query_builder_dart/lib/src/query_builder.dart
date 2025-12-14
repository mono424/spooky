import 'types.dart';
import 'internal/query_logic.dart';

/// Fluent query builder for constructing queries
class QueryBuilder {
  final SchemaStructure schema;
  final String tableName;
  final QueryOptions _options;

  QueryBuilder(this.schema, this.tableName, [QueryOptions? options])
    : _options = options ?? QueryOptions();

  /// Add additional where conditions
  QueryBuilder where(Map<String, dynamic> conditions) {
    final newWhere = Map<String, dynamic>.from(_options.where ?? {});
    newWhere.addAll(conditions);
    return QueryBuilder(schema, tableName, _options.copyWith(where: newWhere));
  }

  /// Specify fields to select
  QueryBuilder select(List<String> fields) {
    if (_options.select != null) {
      throw Exception("Select can only be called once per query");
    }
    return QueryBuilder(schema, tableName, _options.copyWith(select: fields));
  }

  /// Add ordering to the query
  QueryBuilder orderBy(String field, [String direction = 'asc']) {
    final newOrderBy = Map<String, String>.from(_options.orderBy ?? {});
    newOrderBy[field] = direction;
    return QueryBuilder(
      schema,
      tableName,
      _options.copyWith(orderBy: newOrderBy),
    );
  }

  /// Add limit to the query
  QueryBuilder limit(int count) {
    return QueryBuilder(schema, tableName, _options.copyWith(limit: count));
  }

  /// Add offset to the query
  QueryBuilder offset(int count) {
    return QueryBuilder(schema, tableName, _options.copyWith(offset: count));
  }

  /// Mark query to return single result
  QueryBuilder one() {
    return QueryBuilder(schema, tableName, _options.copyWith(isOne: true));
  }

  /// Include related data via subqueries
  QueryBuilder related(
    String field, {
    Function(QueryBuilder)? modifier,
    Cardinality? cardinality,
  }) {
    // Check if field already exists
    if (_options.related?.any((r) => r.alias == field) ?? false) {
      return this;
    }

    // Look up relationship metadata
    final relationship = schema.getRelationship(tableName, field);
    if (relationship == null) {
      throw Exception("Relationship '$field' not found for table '$tableName'");
    }

    // Determine cardinality
    final actualCardinality = cardinality ?? relationship.cardinality;

    // Determine foreign key field
    String foreignKeyField;
    if (actualCardinality == Cardinality.many) {
      // Logic to find reverse relationship
      // Simplified for Dart port: assume standard naming or look up reverse
      // For now, defaulting to tableName (parent table name)
      foreignKeyField = tableName;
    } else {
      foreignKeyField = field;
    }

    final newRelated = List<RelatedQuery>.from(_options.related ?? []);
    newRelated.add(
      RelatedQuery(
        relatedTable: relationship.to,
        alias: field,
        modifier: modifier,
        cardinality: actualCardinality,
        foreignKeyField: foreignKeyField,
      ),
    );

    return QueryBuilder(
      schema,
      tableName,
      _options.copyWith(related: newRelated),
    );
  }

  /// Build the final query string
  QueryInfo build() {
    return buildQueryFromOptions("SELECT", tableName, _options, schema);
  }

  /// Get the generated SQL string
  String toSql() {
    return build().query;
  }
}
