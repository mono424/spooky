import '../utils/duration_utils.dart';

/// A comparison operator condition for [QueryBuilder.where], mirroring the JS
/// `{ _op, _val, _swap }` shape (e.g. `QueryOp('>', 18)` -> `field > $field`).
class QueryOp {
  const QueryOp(this.op, this.value, {this.swap = false});
  final String op;
  final Object? value;
  final bool swap;
}

/// Registers a built query and returns its hash.
typedef QueryRegistrar = Future<String> Function(
    String surql, Map<String, dynamic> vars, QueryTimeToLive ttl);

/// Subscribes to a registered query hash as a Stream.
typedef QuerySubscriber = Stream<List<Map<String, dynamic>>> Function(
    String hash);

/// Fluent SURQL query builder. Produces the same SURQL shape as the JS
/// `buildQueryFromOptions` for select/where/orderBy/limit/offset/one. Relation
/// FETCH subqueries are not supported here.
class QueryBuilder {
  QueryBuilder(this._table,
      {QueryRegistrar? registrar, QuerySubscriber? subscriber})
      : _registrar = registrar,
        _subscriber = subscriber;

  final String _table;
  final QueryRegistrar? _registrar;
  final QuerySubscriber? _subscriber;

  List<String> _select = const ['*'];
  final Map<String, Object?> _where = {};
  final List<(String field, String dir)> _orderBy = [];
  int? _limit;
  int? _offset;
  bool _isOne = false;

  QueryBuilder select(List<String> fields) {
    _select = fields.isEmpty ? const ['*'] : fields;
    return this;
  }

  /// Add equality / operator conditions (AND-joined). A value of [QueryOp]
  /// emits `field <op> $field`; any other value emits `field = $field`.
  QueryBuilder where(Map<String, Object?> conditions) {
    _where.addAll(conditions);
    return this;
  }

  QueryBuilder orderBy(String field, [String direction = 'ASC']) {
    _orderBy.add((field, direction));
    return this;
  }

  QueryBuilder limit(int count) {
    _limit = count;
    return this;
  }

  QueryBuilder offset(int count) {
    _offset = count;
    return this;
  }

  QueryBuilder one() {
    _isOne = true;
    return this;
  }

  /// Build the SURQL string and its bind vars.
  (String, Map<String, dynamic>) build() {
    final effectiveLimit = _isOne ? 1 : _limit;
    final selectClause = _select.join(', ');
    var query = 'SELECT $selectClause FROM $_table';

    final vars = <String, dynamic>{};
    if (_where.isNotEmpty) {
      final conditions = <String>[];
      _where.forEach((key, value) {
        if (value is QueryOp) {
          String rightSide;
          if (value.value is String &&
              (value.value as String).startsWith(r'$')) {
            rightSide = value.value as String;
          } else {
            vars[key] = value.value;
            rightSide = '\$$key';
          }
          conditions.add(value.swap
              ? '$rightSide ${value.op} $key'
              : '$key ${value.op} $rightSide');
        } else {
          vars[key] = value;
          conditions.add('$key = \$$key');
        }
      });
      query += ' WHERE ${conditions.join(' AND ')}';
    }

    if (_orderBy.isNotEmpty) {
      final clauses = _orderBy.map((o) => '${o.$1} ${o.$2}').join(', ');
      query += ' ORDER BY $clauses';
    }
    if (effectiveLimit != null) query += ' LIMIT $effectiveLimit';
    if (_offset != null) query += ' START $_offset';

    query += ';';
    return (query, vars);
  }

  /// Register the query and return its hash (TS `.run()`).
  Future<String> run({QueryTimeToLive ttl = defaultTtl}) {
    final registrar = _registrar;
    if (registrar == null) {
      throw StateError('QueryBuilder has no registrar (build-only)');
    }
    final (sql, vars) = build();
    return registrar(sql, vars, ttl);
  }

  /// Register the query and return a result Stream (consume with StreamBuilder).
  Future<Stream<List<Map<String, dynamic>>>> stream(
      {QueryTimeToLive ttl = defaultTtl}) async {
    final subscriber = _subscriber;
    if (subscriber == null) {
      throw StateError('QueryBuilder has no subscriber (build-only)');
    }
    final hash = await run(ttl: ttl);
    return subscriber(hash);
  }
}
