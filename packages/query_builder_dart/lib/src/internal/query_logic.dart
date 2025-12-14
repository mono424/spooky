import '../types.dart';

/// Build a query string from query options
QueryInfo buildQueryFromOptions(
  String method, // SELECT, LIVE SELECT, etc.
  String tableName,
  QueryOptions options,
  SchemaStructure schema,
) {
  if (options.isOne) {
    options.limit = 1;
  }

  // Build SELECT clause
  String selectClause = "*";
  if (options.select != null && options.select!.isNotEmpty) {
    selectClause = options.select!.join(", ");
  }

  // Build WHERE clause
  String whereClause = "";
  if (options.where != null && options.where!.isNotEmpty) {
    final conditions = <String>[];
    options.where!.forEach((key, value) {
      // Simple value handling for now.
      // In a real implementation, we'd need robust value serialization.
      // For strings, wrap in quotes. For numbers/bools, toString.
      String valStr;
      if (value is String) {
        valStr = "'$value'";
      } else {
        valStr = value.toString();
      }

      // Handle special operators if value is a map (e.g. {_op: "∋", ...})
      if (value is Map && value.containsKey('_op')) {
        final op = value['_op'];
        final val = value['_val'];
        final swap = value['_swap'] == true;
        if (swap) {
          conditions.add("$val $op $key");
        } else {
          conditions.add("$key $op $val");
        }
      } else {
        conditions.add("$key = $valStr");
      }
    });
    if (conditions.isNotEmpty) {
      whereClause = "WHERE ${conditions.join(" AND ")}";
    }
  }

  // Build ORDER BY clause
  String orderByClause = "";
  if (options.orderBy != null && options.orderBy!.isNotEmpty) {
    final orders = <String>[];
    options.orderBy!.forEach((field, dir) {
      orders.add("$field ${dir.toUpperCase()}");
    });
    orderByClause = "ORDER BY ${orders.join(", ")}";
  }

  // Build LIMIT and OFFSET
  String limitClause = "";
  if (options.limit != null) {
    limitClause = "LIMIT ${options.limit}";
  }

  String offsetClause = "";
  if (options.offset != null) {
    offsetClause = "START ${options.offset}";
  }

  // Combine parts
  // Note: Subqueries (FETCH/related) are handled separately or via FETCH clause.
  // The TS implementation uses subqueries.

  // If we have related fields, we might need to use subqueries in the projection
  // or separate queries. The TS implementation seems to build a complex object.
  // For this port, let's stick to the basic SELECT first.

  // Wait, the TS implementation builds subqueries in `extractSubqueryQueryInfos`.
  // But `buildQueryFromOptions` just builds the main query.

  // Combine parts
  final parts = [
    method,
    selectClause,
    "FROM",
    tableName,
    if (whereClause.isNotEmpty) whereClause,
    if (orderByClause.isNotEmpty) orderByClause,
    if (limitClause.isNotEmpty) limitClause,
    if (offsetClause.isNotEmpty) offsetClause,
  ];

  final query = parts.join(" ");

  return QueryInfo(query.trim());
}

/// Extract subqueries for related fields
List<InnerQuery> extractSubqueryQueryInfos(
  SchemaStructure schema,
  String parentTableName,
  QueryOptions options,
) {
  if (options.related == null || options.related!.isEmpty) {
    return [];
  }

  return options.related!.map((rel) {
    // We need a builder to get the sub-options.
    // Since we don't have the builder instance here, we assume the modifier
    // has already been applied or we create a temporary one.
    // In Dart, we might need a different approach.
    // Let's assume `rel.modifier` is a function we can call.

    // For now, let's just create a basic sub-query option set.
    var subOptions = QueryOptions();

    // If modifier is provided, we'd need a way to run it.
    // This is tricky without the full Builder class here.
    // We'll defer this logic to the QueryBuilder class which calls this.

    // Logic to determine foreign key and add WHERE clause
    // This mirrors the TS logic

    String foreignKeyField = rel.foreignKeyField;

    // Add parent filter to where clause
    subOptions.where = subOptions.where ?? {};

    if (rel.cardinality == Cardinality.many) {
      // One-to-Many: Child has foreign key to parent
      // WHERE $parentIds ∋ child.parent_id
      subOptions.where![foreignKeyField] = {
        '_op': "INSIDE",
        '_val': r"$parentIds",
        '_swap': true,
      };
    } else {
      // One-to-One: Parent has foreign key to child
      // WHERE $parent_<foreignKeyField> ∋ child.id
      subOptions.where!['id'] = {
        '_op': "INSIDE",
        '_val': "\$parent_$foreignKeyField",
        '_swap': true,
      };
    }

    return InnerQuery(rel.relatedTable, subOptions, schema);
  }).toList();
}

class InnerQuery {
  final String tableName;
  final QueryOptions options;
  final SchemaStructure schema;
  late final QueryInfo selectQuery;

  InnerQuery(this.tableName, this.options, this.schema) {
    selectQuery = buildQueryFromOptions("SELECT", tableName, options, schema);
  }
}
