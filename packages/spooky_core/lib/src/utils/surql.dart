import '../types.dart' show MutationEventType;

/// A multi-statement transaction body (TS `TxQuery`).
class TxQuery {
  const TxQuery({required this.sql, required this.statementCount});
  final String sql;
  final int statementCount;
}

/// A query plus a result extractor for one statement of a transaction
/// (TS `SealedQuery<T>`).
class SealedQuery<T> {
  const SealedQuery({required this.sql, required this.extract});
  final String sql;
  final T Function(List<dynamic> results) extract;
}

/// One item of a `SET` / `RETURN` clause. Mirrors the TS union
/// `{ key, variable } | { statement } | string`.
sealed class SetItem {
  const SetItem();

  /// `field = $field` (the bare-string TS branch).
  factory SetItem.field(String field) = _RawField;

  /// `key = $variable`.
  factory SetItem.keyVar(String key, String variable) = _KeyVar;

  /// A raw statement fragment, inserted verbatim.
  factory SetItem.statement(String statement) = _Statement;

  String render();
}

class _RawField extends SetItem {
  const _RawField(this.field);
  final String field;
  @override
  String render() => '$field = \$$field';
}

class _KeyVar extends SetItem {
  const _KeyVar(this.key, this.variable);
  final String key;
  final String variable;
  @override
  String render() => '$key = \$$variable';
}

class _Statement extends SetItem {
  const _Statement(this.statement);
  final String statement;
  @override
  String render() => statement;
}

/// One item of a `WHERE ... AND` clause. Mirrors `{ field, variable } | string`.
sealed class WhereItem {
  const WhereItem();
  factory WhereItem.field(String field) = _RawWhere;
  factory WhereItem.fieldVar(String field, String variable) = _WhereVar;
  String render();
}

class _RawWhere extends WhereItem {
  const _RawWhere(this.field);
  final String field;
  @override
  String render() => '$field = \$$field';
}

class _WhereVar extends WhereItem {
  const _WhereVar(this.field, this.variable);
  final String field;
  final String variable;
  @override
  String render() => '$field = \$$variable';
}

/// One item of a `SELECT` return list. Mirrors `{ field, alias } | string`.
sealed class ReturnItem {
  const ReturnItem();
  factory ReturnItem.raw(String value) = _RawReturn;
  factory ReturnItem.aliased(String field, String alias) = _AliasedReturn;
  String render();
}

class _RawReturn extends ReturnItem {
  const _RawReturn(this.value);
  final String value;
  @override
  String render() => value;
}

class _AliasedReturn extends ReturnItem {
  const _AliasedReturn(this.field, this.alias);
  final String field;
  final String alias;
  @override
  String render() => '$field as $alias';
}

/// SurrealQL builder helpers. Faithful port of the TS `surql` object.
class Surql {
  const Surql();

  /// Seal a single statement string: appends `;`.
  String seal(String query) => '$query;';

  /// Seal a transaction, returning a [SealedQuery] whose [SealedQuery.extract]
  /// reads the chosen inner statement (defaulting to the last). The `+1`
  /// skips the `BEGIN` null at index 0.
  SealedQuery<dynamic> sealTx(TxQuery query, {int? resultIndex}) {
    final idx = resultIndex ?? query.statementCount - 1;
    return SealedQuery<dynamic>(
      sql: '${query.sql};',
      extract: (results) => results[idx + 1],
    );
  }

  /// Build a `BEGIN/COMMIT` transaction wrapping [queries].
  TxQuery tx(List<String> queries) => TxQuery(
        sql: 'BEGIN TRANSACTION;\n${queries.join(';')};\nCOMMIT TRANSACTION',
        statementCount: queries.length,
      );

  String selectById(String idVar, List<String> returnValues) =>
      'SELECT ${returnValues.join(',')} FROM ONLY \$$idVar';

  String selectByFieldsAnd(
    String table,
    List<WhereItem> whereVar,
    List<ReturnItem> returnValues,
  ) =>
      'SELECT ${returnValues.map((rv) => rv.render()).join(',')} '
      'FROM $table WHERE ${whereVar.map((wv) => wv.render()).join(' AND ')}';

  String create(String idVar, String dataVar) =>
      'CREATE ONLY \$$idVar CONTENT \$$dataVar';

  String createSet(String idVar, List<SetItem> keyDataVars) =>
      'CREATE ONLY \$$idVar SET ${keyDataVars.map((k) => k.render()).join(', ')}';

  String upsert(String idVar, String dataVar) =>
      'UPSERT ONLY \$$idVar REPLACE \$$dataVar';

  /// Sync-down ingestion: MERGE (not REPLACE) so local-only fields
  /// (`_00_crdt`, `_00_cursor`) survive the round-trip.
  String upsertMerge(String idVar, String dataVar) =>
      'UPSERT ONLY \$$idVar MERGE \$$dataVar';

  String updateMerge(String idVar, String dataVar) =>
      'UPDATE ONLY \$$idVar MERGE \$$dataVar';

  String updateSet(String idVar, List<SetItem> keyDataVar) =>
      'UPDATE \$$idVar SET ${keyDataVar.map((k) => k.render()).join(', ')}';

  String delete(String idVar) => 'DELETE \$$idVar';

  String let(String name, String query) => 'LET \$$name = ($query)';

  String createMutation(
    MutationEventType t,
    String mutationIdVar,
    String recordIdVar, {
    String? dataVar,
    String? beforeRecordVar,
  }) {
    switch (t) {
      case MutationEventType.create:
        return "CREATE ONLY \$$mutationIdVar SET mutationType = 'create', "
            'recordId = \$$recordIdVar';
      case MutationEventType.update:
        var stmt = "CREATE ONLY \$$mutationIdVar SET mutationType = 'update', "
            'recordId = \$$recordIdVar, data = \$$dataVar';
        if (beforeRecordVar != null) {
          stmt += ', beforeRecord = \$$beforeRecordVar';
        }
        return stmt;
      case MutationEventType.delete:
        return "CREATE ONLY \$$mutationIdVar SET mutationType = 'delete', "
            'recordId = \$$recordIdVar';
    }
  }

  String returnObject(List<(String key, String variable)> entries) =>
      'RETURN {${entries.map((e) => '${e.$1}: \$${e.$2}').join(',')}}';
}

/// Singleton matching the TS `surql` export.
const surql = Surql();
