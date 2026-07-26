import '../../services/database/local_database_service.dart';
import '../../utils/sort_rows.dart';
import '../query_builder.dart' show RelationPlan;

/// Nesting depth beyond which a relation tree is treated as cyclic
/// (TS `MAX_RELATION_DEPTH`).
const int maxRelationDepth = 12;

/// Thrown when a `.related()` tree nests past [maxRelationDepth]
/// (TS `RelationCycleError`).
class RelationCycleError extends Error {
  RelationCycleError(this.path);
  final List<String> path;

  @override
  String toString() =>
      'RelationCycleError: relation nesting exceeded $maxRelationDepth at ${path.join(' -> ')}';
}

/// Fetches candidate child rows for one relation level.
abstract class RelationFetcher {
  /// Rows of [table] whose [matchField] is one of [keys].
  List<Map<String, dynamic>> fetchRelation({
    required String table,
    required String matchField,
    required List<Object?> keys,
  });
}

/// [RelationFetcher] over the local sqlite store.
///
/// Divergence from the TS core, which pushes the match into a local SurrealQL /
/// SQLite query: this scans the logical table and filters in Dart. The local
/// store is a document table keyed only by `(tbl, id)`, so a field match has no
/// index to use either way; scanning keeps the resolver engine-free. Fine for a
/// client-sized cache — revisit with a JSON1 index if a table ever grows large.
class LocalRelationFetcher implements RelationFetcher {
  LocalRelationFetcher(this._local);

  final LocalDatabaseService _local;

  @override
  List<Map<String, dynamic>> fetchRelation({
    required String table,
    required String matchField,
    required List<Object?> keys,
  }) {
    final wanted = {for (final key in keys) stableKey(key)};
    return [
      for (final row in _local.getAll(table))
        if (wanted.contains(stableKey(row[matchField]))) row,
    ];
  }
}

/// Resolve a query's `.related()` tree by level-ordered, batched fan-out — the
/// engine-neutral replacement for SurrealQL's nested `(SELECT …) AS alias`
/// projections (TS `resolveRelations`). One fetch per relation per level, never
/// per row.
///
/// Correlation mirrors the emitted subquery:
/// - `one`  -> parent[foreignKeyField] == child.id, attach the first match or null
/// - `many` -> child[foreignKeyField] == parent.id, attach the list
///
/// [parents] is mutated in place, each alias appended LAST so key order matches
/// SurrealQL's `SELECT *, <sub> AS alias`.
///
/// Synchronous, unlike the TS core: sqlite reads are synchronous here, and
/// keeping this sync is what lets `DataModule` materialize (and paint) a query
/// without an await.
///
/// Throws [RelationCycleError] when nesting exceeds [maxRelationDepth].
void resolveRelations(
  List<Map<String, dynamic>> parents,
  List<RelationPlan> relations,
  RelationFetcher fetcher, {
  int depth = 0,
  List<String> path = const [],
}) {
  if (relations.isEmpty || parents.isEmpty) return;
  if (depth >= maxRelationDepth) {
    throw RelationCycleError([...path, relations.map((r) => r.alias).join('|')]);
  }
  for (final relation in relations) {
    _resolveOne(parents, relation, fetcher, depth, path);
  }
}

void _resolveOne(
  List<Map<String, dynamic>> parents,
  RelationPlan relation,
  RelationFetcher fetcher,
  int depth,
  List<String> path,
) {
  final isOne = relation.isOne;
  // The child field to match parent keys against.
  final matchField = isOne ? 'id' : relation.foreignKeyField;
  Object? parentKeyOf(Map<String, dynamic> parent) =>
      isOne ? parent[relation.foreignKeyField] : parent['id'];

  // Distinct, non-null correlation keys: an absent foreign key contributes
  // nothing, so it never triggers a spurious match-everything fetch.
  final keys = <String, Object?>{};
  for (final parent in parents) {
    final key = parentKeyOf(parent);
    if (key == null) continue;
    keys.putIfAbsent(stableKey(key), () => key);
  }

  final grouped = <String, List<Map<String, dynamic>>>{};
  if (keys.isNotEmpty) {
    final children = fetcher.fetchRelation(
      table: relation.table,
      matchField: matchField,
      keys: keys.values.toList(),
    );

    // Recurse BEFORE grouping: each child is itself a parent for its own
    // relations, and resolving the deduped child set once keeps nested fan-out
    // at O(depth) batches. Children later dropped by a per-parent limit carry
    // resolved nested data harmlessly — they reach no output row.
    resolveRelations(
      children,
      relation.relations,
      fetcher,
      depth: depth + 1,
      path: [...path, relation.alias],
    );

    for (final child in children) {
      grouped.putIfAbsent(stableKey(child[matchField]), () => []).add(child);
    }
  }

  for (final parent in parents) {
    final key = parentKeyOf(parent);
    var bucket = key == null
        ? <Map<String, dynamic>>[]
        : [...?grouped[stableKey(key)]];

    // Filter, then order, then limit — PER PARENT. A shared batch fetch can't
    // express "top N per parent", so the shaping happens here.
    if (relation.where.isNotEmpty) {
      bucket = [
        for (final row in bucket)
          if (_matchesWhere(row, relation.where)) row,
      ];
    }
    if (relation.orderBy.isNotEmpty) {
      bucket = sortRows(bucket, relation.orderBy);
    }
    final limit = relation.limit;
    if (limit != null && bucket.length > limit) {
      bucket = bucket.sublist(0, limit);
    }

    // Remove first so a re-resolved alias lands LAST in key order, matching
    // SurrealQL's `SELECT *, <sub> AS alias`.
    parent.remove(relation.alias);
    parent[relation.alias] = isOne ? (bucket.isEmpty ? null : bucket.first) : bucket;
  }
}

bool _matchesWhere(Map<String, dynamic> row, Map<String, Object?> where) {
  for (final entry in where.entries) {
    if (stableKey(row[entry.key]) != stableKey(entry.value)) return false;
  }
  return true;
}
