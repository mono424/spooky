import '../../events/event_system.dart';
import '../../services/database/remote_database_service.dart';
import '../../services/logger/logger.dart';
import '../../surreal/value.dart';
import '../../types.dart';
import '../../utils/parser.dart';
import '../../utils/record_id_utils.dart';
import '../cache/cache_module.dart';
import 'sync_events.dart';

/// Fetches remote records for a diff, cleans them, and ingests via the cache
/// (TS `SyncEngine`). Separates "how to sync" from "when to sync".
class SyncEngine {
  SyncEngine(this._remote, this._cache, this._schema, SpookyLogger logger)
      : _logger = logger.child('SyncEngine');

  final RemoteDatabaseService _remote;
  final CacheModule _cache;
  final Map<String, dynamic> _schema;
  final SpookyLogger _logger;
  final EventSystem events = createSyncEventSystem();

  Future<void> syncRecords(RecordVersionDiff diff) async {
    if (diff.removed.isNotEmpty) {
      await _handleRemovedRecords(diff.removed);
    }

    final toFetch = [...diff.added, ...diff.updated];
    if (toFetch.isEmpty) return;

    final idsToFetch = toFetch.map((x) => x.id).toList();
    final versionMap = <String, int>{
      for (final item in toFetch) encodeRecordId(item.id): item.version,
    };

    // The CBOR client sends these record ids as real records, so a bare
    // `FROM $idsToFetch` fetches them directly (matches the TS core).
    final results = await _remote
        .query('SELECT * FROM \$idsToFetch', {'idsToFetch': idsToFetch});
    final remoteResults = firstRows(results).cast<Map<String, dynamic>>();

    final cacheBatch = <CacheRecord>[];
    for (final record in remoteResults) {
      final id = record['id'];
      if (id == null) {
        _logger.warn('Remote record has no id; skipping');
        continue;
      }
      final fullId = id.toString();
      final table = extractTablePart(fullId);
      final isAdded =
          diff.added.any((item) => encodeRecordId(item.id) == fullId);
      final version = versionMap[fullId] ?? 0;

      final localVersion = _cache.lookup(fullId);
      if (localVersion != 0 && version <= localVersion) {
        continue; // local is newer or equal
      }

      final columns = _columnsFor(table);
      final cleaned = columns != null ? cleanRecord(columns, record) : record;

      cacheBatch.add(CacheRecord(
        table: table,
        op: isAdded ? 'CREATE' : 'UPDATE',
        record: cleaned,
        version: version,
      ));
    }

    if (cacheBatch.isNotEmpty) {
      await _cache.saveBatch(cacheBatch);
    }

    events.emit(SyncEventTypes.remoteDataIngested, {'records': remoteResults});
  }

  /// Verify removed records are truly gone upstream before deleting locally.
  Future<void> _handleRemovedRecords(List<RecordId> removed) async {
    final existingRemoteIds = <String>{};
    try {
      // The records to check ARE the FROM target. The CBOR client sends them as
      // real records, so `SELECT id FROM $ids` matches correctly. (We must NOT
      // use `WHERE id IN $ids`: record-id `IN` is broken on SurrealDB v3.1 — it
      // returns no rows even for existing records, which would report every id
      // as gone and delete still-present records. Matches the TS core.)
      final results = await _remote.query('SELECT id FROM \$ids', {'ids': removed});
      final rows = firstRows(results);
      for (final row in rows) {
        final id = (row as Map)['id'];
        if (id != null) existingRemoteIds.add(id.toString());
      }
    } catch (err) {
      // On verification failure, skip deletion (avoid clobbering fresh data).
      _logger.warn('Remote existence check failed, skipping deletion: $err');
      return;
    }

    for (final recordId in removed) {
      final idStr = encodeRecordId(recordId);
      if (!existingRemoteIds.contains(idStr)) {
        await _cache.delete(recordId.table, idStr);
      }
    }
  }

  Map<String, ColumnSchema>? _columnsFor(String table) {
    final t = _schema[table];
    if (t == null) return null;
    final columns =
        (t is Map && t['columns'] is Map) ? t['columns'] as Map : (t as Map);
    return columns.map((k, v) => MapEntry(k as String, v as ColumnSchema));
  }
}
