import 'dart:convert';
import 'table_schema.dart';
import 'types.dart';

// Helper to parse strings to RecordId
dynamic parseStringToRecordId(
  dynamic value, {
  String? tableName,
  String? fieldName,
}) {
  if (value is! String) return value;

  // If it already contains ":", parse it as a full record ID
  if (value.contains(':')) {
    final parts = value.split(':');
    final table = parts[0];
    final id = parts.sublist(1).join(':');
    return RecordId(table, id);
  }

  // If this is an "id" field and we have a table name, prepend it
  if (fieldName == 'id' && tableName != null) {
    return RecordId(tableName, value);
  }

  return value;
}

// Recursively parse string IDs to RecordId
dynamic parseObjectIdsToRecordId(dynamic obj, {String? tableName}) {
  if (obj == null) return obj;

  if (obj is String) {
    return parseStringToRecordId(obj, tableName: tableName);
  }

  if (obj is List) {
    return obj
        .map((item) => parseObjectIdsToRecordId(item, tableName: tableName))
        .toList();
  }

  if (obj is Map<String, dynamic>) {
    final result = <String, dynamic>{};
    for (final entry in obj.entries) {
      final key = entry.key;
      final value = entry.value;
      result[key] = value is String
          ? parseStringToRecordId(value, tableName: tableName, fieldName: key)
          : parseObjectIdsToRecordId(value, tableName: tableName);
    }
    return result;
  }

  return obj;
}

typedef Executor<R> = R Function(InnerQuery query);

class InnerQuery<R> {
  late int _hash;
  late QueryInfo _mainQuery;
  late QueryInfo _selectQuery;
  late QueryInfo _selectLiveQuery;
  late List<InnerQuery<dynamic>> _subqueries;

  final String _tableName;
  final QueryOptions options;
  final SchemaStructure schema;
  final Executor<R> executor;

  InnerQuery(this._tableName, this.options, this.schema, this.executor) {
    _selectQuery = buildQueryFromOptions('SELECT', _tableName, options, schema);

    final mainOptions = options.copyWith(related: []);
    _mainQuery = buildQueryFromOptions(
      'SELECT',
      _tableName,
      mainOptions,
      schema,
    );

    _hash = _selectQuery.hash;

    _selectLiveQuery = buildQueryFromOptions(
      'LIVE SELECT',
      _tableName,
      options,
      schema,
    );

    _subqueries = extractSubqueryQueryInfos(
      schema,
      _tableName,
      options,
      (q) => null,
    );
  }

  QueryInfo get mainQuery => _mainQuery;
  List<InnerQuery<dynamic>> get subqueries => _subqueries;
  QueryInfo get selectQuery => _selectQuery;
  QueryInfo get selectLiveQuery => _selectLiveQuery;
  String get tableName => _tableName;
  int get hash => _hash;
  bool get isOne => options.isOne;

  R run() {
    return executor(this);
  }

  QueryInfo buildUpdateQuery(List<dynamic> patches) {
    return buildQueryFromOptions(
      'UPDATE',
      _tableName,
      options,
      schema,
      patches: patches,
    );
  }

  QueryInfo buildDeleteQuery() {
    return buildQueryFromOptions('DELETE', _tableName, options, schema);
  }

  QueryOptions getOptions() {
    return options;
  }
}

class FinalQuery<R> {
  late InnerQuery<R> _innerQuery;
  final String tableName;
  final QueryOptions options;
  final SchemaStructure schema;
  final Executor<R> executor;

  FinalQuery(this.tableName, this.options, this.schema, this.executor) {
    _innerQuery = InnerQuery(tableName, options, schema, executor);
  }

  R run() {
    return executor(_innerQuery);
  }

  QueryInfo buildUpdateQuery(List<dynamic> patches) {
    return _innerQuery.buildUpdateQuery(patches);
  }

  QueryInfo buildDeleteQuery() {
    return _innerQuery.buildDeleteQuery();
  }

  QueryInfo selectLive() {
    return _innerQuery.selectLiveQuery;
  }

  InnerQuery<R> get innerQuery => _innerQuery;
  bool get isOne => options.isOne;
  int get hash => _innerQuery.hash;
}

class SchemaAwareQueryModifierBuilderImpl implements QueryModifierBuilder {
  final String tableName;
  final SchemaStructure schema;
  final QueryOptions options = QueryOptions();

  SchemaAwareQueryModifierBuilderImpl(this.tableName, this.schema);

  @override
  QueryModifierBuilder where(Map<String, dynamic> conditions) {
    options.where ??= {};
    options.where!.addAll(conditions);
    return this;
  }

  @override
  QueryModifierBuilder select(List<String> fields) {
    if (options.select != null) {
      throw Exception('Select can only be called once per query');
    }
    options.select = fields;
    return this;
  }

  @override
  QueryModifierBuilder limit(int count) {
    options.limit = count;
    return this;
  }

  @override
  QueryModifierBuilder offset(int count) {
    options.offset = count;
    return this;
  }

  @override
  QueryModifierBuilder orderBy(String field, {String direction = "asc"}) {
    options.orderBy ??= {};
    options.orderBy![field] = direction;
    return this;
  }

  @override
  QueryModifierBuilder related(
    String relatedField, {
    QueryModifier? modifier,
    Cardinality? cardinality,
  }) {
    options.related ??= [];

    final exists = options.related!.any(
      (r) => (r.alias ?? r.relatedTable) == relatedField,
    );

    if (!exists) {
      // Look up relationship
      final relationship = schema.relationships.firstWhere(
        (r) => r.from == tableName && r.field == relatedField,
        orElse: () => throw Exception(
          "Relationship '$relatedField' not found for table '$tableName'",
        ),
      );

      final relatedTable = relationship.to;
      final actualCardinality = cardinality ?? relationship.cardinality;
      final foreignKeyField = actualCardinality == Cardinality.many
          ? tableName
          : relatedField;

      options.related!.add(
        RelatedQuery(
          relatedTable: relatedTable,
          alias: relatedField,
          modifier: modifier,
          cardinality: actualCardinality,
          foreignKeyField: foreignKeyField,
        ),
      );
    }

    return this;
  }

  @override
  QueryOptions getOptions() {
    return options;
  }
}

class QueryBuilder<R> {
  final SchemaStructure schema;
  final String tableName;
  final Executor<R> executor;
  final QueryOptions options;

  QueryBuilder(
    this.schema,
    this.tableName, {
    required this.executor,
    QueryOptions? options,
  }) : options = options ?? QueryOptions();

  QueryBuilder<R> where(Map<String, dynamic> conditions) {
    options.where ??= {};
    options.where!.addAll(conditions);
    return this;
  }

  QueryBuilder<R> select(List<String> fields) {
    if (options.select != null) {
      throw Exception('Select can only be called once per query');
    }
    options.select = fields;
    return this;
  }

  QueryBuilder<R> orderBy(String field, {String direction = 'asc'}) {
    options.orderBy ??= {};
    options.orderBy![field] = direction;
    return this;
  }

  QueryBuilder<R> limit(int count) {
    options.limit = count;
    return this;
  }

  QueryBuilder<R> offset(int count) {
    options.offset = count;
    return this;
  }

  QueryBuilder<dynamic> one() {
    // Note: This changes the return type conceptually, but in Dart restricted generics
    // we keep using QueryBuilder. The result of run() will depend on executor handling options.isOne.
    return QueryBuilder<dynamic>(
      schema,
      tableName,
      executor: (q) => executor(q),
      options: options.copyWith(isOne: true),
    );
  }

  QueryBuilder<R> related(
    String field, {
    QueryModifier? modifier,
    Cardinality? cardinality,
  }) {
    options.related ??= [];

    final exists = options.related!.any(
      (r) => (r.alias ?? r.relatedTable) == field,
    );

    if (exists) return this;

    final relationship = schema.relationships.firstWhere(
      (r) => r.from == tableName && r.field == field,
      orElse: () => throw Exception(
        "Relationship '$field' not found for table '$tableName'",
      ),
    );

    final actualCardinality = cardinality ?? relationship.cardinality;

    // Determine foreign key field
    String foreignKeyField = actualCardinality == Cardinality.many
        ? tableName
        : field;

    if (actualCardinality == Cardinality.many) {
      // Look for reverse relationship
      final reverseRelationships = schema.relationships
          .where(
            (r) =>
                r.from == relationship.to &&
                r.to == tableName &&
                r.cardinality == Cardinality.one,
          )
          .toList();

      if (reverseRelationships.isNotEmpty) {
        final exactMatch = reverseRelationships.firstWhere(
          (r) => r.field == tableName,
          orElse: () => reverseRelationships.first,
        );
        foreignKeyField = exactMatch.field;
      } else {
        // Fallback
        if (tableName.startsWith('${relationship.to}_')) {
          foreignKeyField = tableName.substring(relationship.to.length + 1);
        }
      }
    }

    options.related!.add(
      RelatedQuery(
        relatedTable: relationship.to,
        alias: field,
        modifier: modifier,
        cardinality: actualCardinality,
        foreignKeyField: foreignKeyField,
      ),
    );

    return this;
  }

  QueryOptions getOptions() {
    return options;
  }

  FinalQuery<R> build() {
    return FinalQuery(tableName, options, schema, executor);
  }
}

int cyrb53(String str, [int seed = 0]) {
  int h1 = 0xdeadbeef ^ seed;
  int h2 = 0x41c6ce57 ^ seed;
  for (int i = 0; i < str.length; i++) {
    int ch = str.codeUnitAt(i);
    h1 = (h1 ^ ch) * 2654435761;
    h2 = (h2 ^ ch) * 1597334677;
    // JS Math.imul simulation
    h1 = h1 & 0xFFFFFFFF;
    h2 = h2 & 0xFFFFFFFF;
  }

  h1 = ((h1 ^ (h1 >> 16)) * 2246822507) & 0xFFFFFFFF;
  h1 = h1 ^ ((h2 ^ (h2 >> 13)) * 3266489909) & 0xFFFFFFFF;

  h2 = ((h2 ^ (h2 >> 16)) * 2246822507) & 0xFFFFFFFF;
  h2 = h2 ^ ((h1 ^ (h1 >> 13)) * 3266489909) & 0xFFFFFFFF;

  return (4294967296 * (2097151 & h2) + (h1 >> 0)).toInt();
}

List<InnerQuery<dynamic>> extractSubqueryQueryInfos(
  SchemaStructure schema,
  String parentTableName,
  QueryOptions options,
  Executor<dynamic> executor,
) {
  if (options.related == null) return [];

  return options.related!.map((rel) {
    final subOptionsBuilder = SchemaAwareQueryModifierBuilderImpl(
      rel.relatedTable,
      schema,
    );

    if (rel.modifier != null) {
      rel.modifier!(subOptionsBuilder);
    }

    final subOptions = subOptionsBuilder.getOptions();

    // Logic to find relationship and inject filter
    final relationship = schema.relationships.firstWhere(
      (r) => r.from == parentTableName && r.field == rel.alias,
      orElse: () => throw Exception('Relationship not found (internal error)'),
    );

    // rel.alias is guaranteed
    String foreignKeyField = rel.foreignKeyField ?? rel.alias!;
    // Note: Logic for foreignKeyField override is in the Builder.related methods in TS.
    // Here we should rely on what was stored in RelatedQuery if populated,
    // or re-derive if needed.
    // In TS `buildSubquery` logic is duplicated/shared.
    // Check `extractSubqueryQueryInfos` in TS. It re-derives `foreignKeyField` if not present.
    // Our Builder (above) populates `foreignKeyField` in `RelatedQuery`.

    subOptions.where ??= {};

    if (relationship.cardinality == Cardinality.many) {
      // child.parent_id ∋ $parentIds
      subOptions.where![foreignKeyField] = {
        '_op': '∋',
        '_val': r'$parentIds',
        '_swap': true,
      };
    } else {
      // child.id ∋ $parent_fk
      subOptions.where!['id'] = {
        '_op': '∋',
        '_val': '\$parent_$foreignKeyField',
        '_swap': true,
      };
    }

    return InnerQuery(rel.relatedTable, subOptions, schema, executor);
  }).toList();
}

QueryInfo buildQueryFromOptions(
  String method,
  String tableName,
  QueryOptions options,
  SchemaStructure schema, {
  List<dynamic>? patches,
}) {
  if (options.isOne) {
    options.limit = 1;
  }
  final isLiveQuery = method == 'LIVE SELECT' || method == 'LIVE SELECT DIFF';

  final parsedWhere = options.where != null
      ? parseObjectIdsToRecordId(options.where, tableName: tableName)
            as Map<String, dynamic>
      : null;

  String selectClause = '*';
  if (method == 'LIVE SELECT DIFF') {
    selectClause = '';
  } else {
    if (options.select != null && options.select!.isNotEmpty) {
      selectClause = options.select!.join(', ');
    }
  }

  String fetchClauses = '';
  if (!isLiveQuery && options.related != null && options.related!.isNotEmpty) {
    final subqueries = options.related!.map(
      (rel) => buildSubquery(rel, schema),
    );
    fetchClauses = ', ' + subqueries.join(', ');
  }

  String query = '';
  if (method == 'UPDATE') {
    query = 'UPDATE $tableName';
  } else if (method == 'DELETE') {
    query = 'DELETE FROM $tableName';
  } else {
    query =
        '$method${selectClause.isNotEmpty ? ' $selectClause' : ''}$fetchClauses FROM $tableName';
  }

  Map<String, dynamic> vars = {};
  if (parsedWhere != null && parsedWhere.isNotEmpty) {
    List<String> conditions = [];
    parsedWhere.forEach((key, value) {
      final varName = key; // Simple var mapping

      if (value is Map &&
          value.containsKey('_op') &&
          value.containsKey('_val')) {
        final op = value['_op'];
        final val = value['_val'];
        final swap = value['_swap'] == true;

        String rightSide = '';
        if (val is String && val.startsWith(r'$')) {
          rightSide = val;
        } else {
          vars[varName] = val;
          rightSide = '\$$varName';
        }

        if (swap) {
          conditions.add('$rightSide $op $key');
        } else {
          conditions.add('$key $op $rightSide');
        }
      } else {
        vars[varName] = value;
        conditions.add('$key = \$$varName');
      }
    });
    query += ' WHERE ${conditions.join(' AND ')}';
  }

  if (method == 'UPDATE' && patches != null) {
    query += ' PATCH ${jsonEncode(patches)}';
  }

  if (!isLiveQuery) {
    if (options.orderBy != null && options.orderBy!.isNotEmpty) {
      final orderClauses = options.orderBy!.entries.map(
        (e) => '${e.key} ${e.value}',
      );
      query += ' ORDER BY ${orderClauses.join(', ')}';
    }

    if (options.limit != null) {
      query += ' LIMIT ${options.limit}';
    }

    if (options.offset != null) {
      query += ' START ${options.offset}';
    }
  }

  query += ';';

  return QueryInfo(
    query: query,
    hash: cyrb53('$query::${vars.toString()}'), // Simple hash input
    vars: vars.isNotEmpty ? vars : null,
  );
}

String buildSubquery(RelatedQuery rel, SchemaStructure schema) {
  final foreignKeyField = rel.foreignKeyField ?? rel.alias!;

  String subquerySelect = '*';
  String subqueryWhere = '';
  String subqueryOrderBy = '';
  String subqueryLimit = '';

  if (rel.modifier != null) {
    final modifierBuilder = SchemaAwareQueryModifierBuilderImpl(
      rel.relatedTable,
      schema,
    );
    rel.modifier!(modifierBuilder);
    final subOptions = modifierBuilder.getOptions();

    if (subOptions.select != null && subOptions.select!.isNotEmpty) {
      subquerySelect = subOptions.select!.join(', ');
    }

    if (subOptions.where != null && subOptions.where!.isNotEmpty) {
      final parsedSubWhere =
          parseObjectIdsToRecordId(
                subOptions.where,
                tableName: rel.relatedTable,
              )
              as Map<String, dynamic>;
      final conditions = parsedSubWhere.entries.map((e) {
        if (e.value is RecordId) {
          return '${e.key} = ${e.value}';
        }
        return '${e.key} = ${jsonEncode(e.value)}';
      });
      subqueryWhere = ' AND ${conditions.join(' AND ')}';
    }

    if (subOptions.orderBy != null && subOptions.orderBy!.isNotEmpty) {
      final orderClauses = subOptions.orderBy!.entries.map(
        (e) => '${e.key} ${e.value}',
      );
      subqueryOrderBy = ' ORDER BY ${orderClauses.join(', ')}';
    }

    if (subOptions.limit != null) {
      subqueryLimit = ' LIMIT ${subOptions.limit}';
    }

    if (subOptions.related != null && subOptions.related!.isNotEmpty) {
      // Resolve nested logic... (Simplified for brevity as logic is recursive)
      // In full implementation we need schema lookup again.
      final nestedSubqueries = subOptions.related!.map((nestedRel) {
        // Should resolve schema
        // For now assumes nestedRel is populated or we might miss cardinality/fk without schema lookup.
        // Simulating simple recursion:

        // Looking up schema
        final relationship = schema.relationships.firstWhere(
          (r) =>
              r.from == rel.relatedTable &&
              r.field == (nestedRel.alias ?? nestedRel.relatedTable),
          orElse: () => throw Exception('Nested relationship not found'),
        );

        final nestedForeignKeyField =
            relationship.cardinality == Cardinality.many
            ? rel.relatedTable
            : (nestedRel.alias ?? nestedRel.relatedTable);

        // Re-construct RelatedQuery with info
        final enrichedNested = RelatedQuery(
          relatedTable: relationship.to,
          alias: nestedRel.alias,
          modifier: nestedRel.modifier,
          cardinality: relationship.cardinality,
          foreignKeyField: nestedForeignKeyField,
        );

        return buildSubquery(enrichedNested, schema);
      });

      subquerySelect += ', ' + nestedSubqueries.join(', ');
    }
  }

  String whereCondition;
  if (rel.cardinality == Cardinality.one) {
    whereCondition = 'WHERE id=\$parent.$foreignKeyField';
    if (subqueryLimit.isEmpty) {
      subqueryLimit = ' LIMIT 1';
    }
  } else {
    whereCondition = 'WHERE $foreignKeyField=\$parent.id';
  }

  String subquery =
      '(SELECT $subquerySelect FROM ${rel.relatedTable} $whereCondition$subqueryWhere$subqueryOrderBy$subqueryLimit)';

  if (rel.cardinality == Cardinality.one) {
    subquery += '[0]';
  }

  subquery += ' AS ${rel.alias}';

  return subquery;
}
