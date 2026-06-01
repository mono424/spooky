import '../../ffi/stream_update.dart';
import '../../services/database/local_database_service.dart';
import '../../services/logger/logger.dart';
import '../../services/stream_processor/stream_processor_service.dart';
import '../../surreal/value.dart';

/// A record to persist + ingest into DBSP (TS `CacheRecord`).
class CacheRecord {
  CacheRecord({
    required this.table,
    required this.op,
    required this.record,
    required this.version,
  });

  final String table;

  /// `'CREATE' | 'UPDATE' | 'DELETE'`.
  final String op;

  /// Full record, must include an `id` (string `table:id` or [RecordId]).
  final Map<String, dynamic> record;
  final int version;
}

/// Centralized local storage + DBSP ingestion bridge (TS `CacheModule`).
///
/// Adapted to the sqlite [LocalDatabaseService] document CRUD API: instead of
/// building a SurrealQL `UPSERT ... MERGE` transaction, it calls
/// [LocalDatabaseService.upsertMerge] per record (still MERGE, so local-only
/// `_00_crdt` / `_00_cursor` fields survive sync-down).
class CacheModule implements StreamUpdateReceiver {
  CacheModule(
    this._local,
    this._streamProcessor,
    this._streamUpdateCallback,
    SpookyLogger logger,
  ) : _logger = logger.child('CacheModule') {
    _streamProcessor.addReceiver(this);
  }

  final LocalDatabaseService _local;
  final StreamProcessorService _streamProcessor;
  final void Function(StreamUpdate update) _streamUpdateCallback;
  // ignore: unused_field
  final SpookyLogger _logger;
  final Map<String, int> _versionLookups = {};

  @override
  void onStreamUpdate(StreamUpdate update) => _streamUpdateCallback(update);

  /// Current known version for a record id (TS `lookup`).
  int lookup(String recordId) => _versionLookups[recordId] ?? 0;

  Future<void> save(CacheRecord record, {bool skipDbInsert = false}) =>
      saveBatch([record], skipDbInsert: skipDbInsert);

  /// Persist records to local and ingest each into DBSP.
  Future<void> saveBatch(List<CacheRecord> records,
      {bool skipDbInsert = false}) async {
    if (records.isEmpty) return;

    for (final record in records) {
      final recordId = _idString(record.record['id']);
      final content = {...record.record, '_00_rv': record.version};

      if (!skipDbInsert) {
        // MERGE, not REPLACE: preserve local-only fields omitted by the
        // remote payload (`_00_crdt`, `_00_cursor`).
        _local.upsertMerge(recordId, content);
      }

      _versionLookups[recordId] = record.version;
      _streamProcessor.ingest(record.table, record.op, recordId, content);
    }
  }

  /// Delete a record from local and ingest the deletion into DBSP.
  Future<void> delete(
    String table,
    String id, {
    bool skipDbDelete = false,
    Map<String, dynamic> recordData = const {},
  }) async {
    if (!skipDbDelete) {
      _local.delete(id);
    }
    _versionLookups.remove(id);
    _streamProcessor.ingest(table, 'DELETE', id, recordData);
  }

  /// Register a query plan with DBSP and return its initial `localArray`.
  RecordVersionArray registerQuery(QueryPlanConfig config) {
    final update = _streamProcessor.registerQueryPlan(config);
    if (update == null) {
      throw StateError('Failed to register query with DBSP');
    }
    return update.localArray;
  }

  void unregisterQuery(String queryHash) =>
      _streamProcessor.unregisterQueryPlan(queryHash);

  String _idString(Object? id) {
    if (id is RecordId) return id.encode();
    return id.toString();
  }
}
