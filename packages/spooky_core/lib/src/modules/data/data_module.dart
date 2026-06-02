import 'dart:async';
import 'dart:convert';

import 'package:crypto/crypto.dart';

import '../../events/event_system.dart' show PushEventOptions, DebouncedConfig;
import '../../ffi/stream_update.dart';
import '../../services/database/local_database_service.dart';
import '../../services/logger/logger.dart';
import '../../services/stream_processor/stream_processor_service.dart';
import '../../surreal/value.dart';
import '../../types.dart';
import '../../utils/duration_utils.dart';
import '../../utils/parser.dart';
import '../../utils/record_id_utils.dart';
import '../cache/cache_module.dart';
import '../sync/queue/queue_up.dart';

/// Unified query and mutation management (TS `DataModule`).
///
/// Read-path divergence from the JS core: the JS re-runs the registered SURQL
/// against the local SurrealDB after each stream update to fetch record bodies.
/// sqlite can't run SurrealQL, so this resolves the result set from the DBSP
/// `localArray` (`[id, version]` pairs) via [LocalDatabaseService.getById].
class DataModule {
  DataModule(
    this._cache,
    this._local,
    this._schema,
    SpookyLogger logger, {
    int streamDebounceTime = 100,
    void Function(String hash)? onHeartbeat,
  })  : _logger = logger.child('DataModule'),
        _streamDebounceTime = streamDebounceTime,
        _onHeartbeat = onHeartbeat;

  final CacheModule _cache;
  final LocalDatabaseService _local;
  final Map<String, dynamic>
      _schema; // table -> { columns: {name: ColumnSchema} }
  final SpookyLogger _logger;
  final int _streamDebounceTime;

  /// Called when a query's TTL heartbeat fires (TS leaves this a TODO; here it
  /// is wired so the client can re-register/heartbeat the query before the
  /// server expires it). Null in local-only mode (timer just reschedules).
  final void Function(String hash)? _onHeartbeat;

  final Map<String, QueryState> _activeQueries = {};
  final Map<String, Future<String>> _pendingQueries = {};
  final Map<String, Set<QueryUpdateCallback>> _subscriptions = {};
  final Set<MutationCallback> _mutationCallbacks = {};
  final Map<String, Timer> _debounceTimers = {};

  String _sessionId = '';
  String? _currentUserId;

  Future<void> init(String sessionId) async {
    _sessionId = sessionId;
  }

  void setSessionId(String sessionId) => _sessionId = sessionId;
  void setCurrentUserId(String? userId) => _currentUserId = userId;
  String? getCurrentUserId() => _currentUserId;

  // ==================== QUERY MANAGEMENT ====================

  /// Register a query and return its hash (TS `query`).
  Future<String> query(
    String tableName,
    String surqlString,
    Map<String, dynamic> params,
    QueryTimeToLive ttl,
  ) async {
    final hash = calculateHash({'surql': surqlString, 'params': params});
    final recordId = RecordId('_00_query', hash);

    if (_activeQueries.containsKey(hash)) return hash;

    final pending = _pendingQueries[hash];
    if (pending != null) {
      await pending;
      return hash;
    }

    final promise = _createAndRegisterQuery(
        hash, recordId, surqlString, params, ttl, tableName);
    _pendingQueries[hash] = promise;
    try {
      await promise;
    } finally {
      _pendingQueries.remove(hash);
    }
    return hash;
  }

  /// Subscribe to query updates (TS `subscribe`). Returns an unsubscribe fn.
  void Function() subscribe(
    String queryHash,
    QueryUpdateCallback callback, {
    bool immediate = false,
  }) {
    final subs = _subscriptions.putIfAbsent(queryHash, () => {});
    subs.add(callback);

    if (immediate) {
      final query = _activeQueries[queryHash];
      if (query != null) callback(query.records);
    }

    return () {
      final s = _subscriptions[queryHash];
      if (s != null) {
        s.remove(callback);
        if (s.isEmpty) _subscriptions.remove(queryHash);
      }
    };
  }

  /// Subscribe to mutations (TS `onMutation`).
  void Function() onMutation(MutationCallback callback) {
    _mutationCallbacks.add(callback);
    return () => _mutationCallbacks.remove(callback);
  }

  /// Handle a stream update from DBSP (TS `onStreamUpdate`). UPDATE is
  /// debounced; CREATE/DELETE propagate immediately.
  Future<void> onStreamUpdate(StreamUpdate update) async {
    if (update.op == 'UPDATE') {
      _debounceTimers[update.queryHash]?.cancel();
      _debounceTimers[update.queryHash] =
          Timer(Duration(milliseconds: _streamDebounceTime), () {
        _debounceTimers.remove(update.queryHash);
        _processStreamUpdate(update);
      });
    } else {
      await _processStreamUpdate(update);
    }
  }

  Future<void> _processStreamUpdate(StreamUpdate update) async {
    final queryState = _activeQueries[update.queryHash];
    if (queryState == null) {
      _logger.warn('Update for unknown query ${update.queryHash}; skipping');
      return;
    }

    final ms = update.materializationTimeMs;
    if (ms != null) {
      queryState.materializationSamples.add(ms);
      if (queryState.materializationSamples.length >
          materializationSampleWindow) {
        queryState.materializationSamples.removeAt(0);
      }
      queryState.lastIngestLatencyMs = ms;
    }

    try {
      final newRecords = _materialize(update.localArray);
      queryState.config.localArray = update.localArray;

      final prevJson = jsonEncode(queryState.records);
      final newJson = jsonEncode(newRecords);
      queryState.records = newRecords;
      final recordsChanged = prevJson != newJson;
      if (recordsChanged) queryState.updateCount++;

      _persistQueryStats(queryState);

      if (!recordsChanged) return;
      _notify(update.queryHash, queryState.records);
    } catch (err) {
      queryState.errorCount++;
      _logger.error('Failed to materialize stream update', err);
      _local.patchQueryConfig(encodeRecordId(queryState.config.id),
          {'errorCount': queryState.errorCount});
    }
  }

  /// Resolve `[id, version]` pairs to full record bodies from local sqlite.
  List<Map<String, dynamic>> _materialize(RecordVersionArray localArray) {
    final out = <Map<String, dynamic>>[];
    for (final pair in localArray) {
      final record = _local.getById(pair.$1);
      if (record != null) out.add(record);
    }
    return out;
  }

  void _persistQueryStats(QueryState qs) {
    final p = _computePercentiles(qs.materializationSamples);
    _local.patchQueryConfig(encodeRecordId(qs.config.id), {
      'localArray': _encodeVersionArray(qs.config.localArray),
      'rowCount': qs.config.localArray.length,
      'updateCount': qs.updateCount,
      'lastIngestLatency': qs.lastIngestLatencyMs,
      'materializationP55': p.$1,
      'materializationP90': p.$2,
      'materializationP99': p.$3,
    });
  }

  (double?, double?, double?) _computePercentiles(List<double> samples) {
    if (samples.isEmpty) return (null, null, null);
    final sorted = [...samples]..sort();
    double pick(double q) {
      final idx = (q * sorted.length).floor();
      return sorted[idx >= sorted.length ? sorted.length - 1 : idx];
    }

    return (pick(0.55), pick(0.90), pick(0.99));
  }

  QueryState? getQueryByHash(String hash) => _activeQueries[hash];
  QueryState? getQueryById(RecordId id) => _activeQueries[extractIdPart(id)];
  List<QueryState> getActiveQueries() => _activeQueries.values.toList();
  List<String> getActiveQueryHashes() => _activeQueries.keys.toList();

  /// Persist a query's local version array (TS `updateQueryLocalArray`).
  Future<void> updateQueryLocalArray(
      String id, RecordVersionArray localArray) async {
    final qs = _activeQueries[id];
    if (qs == null) return;
    qs.config.localArray = localArray;
    _local.patchQueryConfig(encodeRecordId(qs.config.id),
        {'localArray': _encodeVersionArray(localArray)});
  }

  /// Persist a query's remote version array (TS `updateQueryRemoteArray`).
  Future<void> updateQueryRemoteArray(
      String hash, RecordVersionArray remoteArray) async {
    final qs = getQueryByHash(hash);
    if (qs == null) return;
    qs.config.remoteArray = remoteArray;
    _local.patchQueryConfig(encodeRecordId(qs.config.id),
        {'remoteArray': _encodeVersionArray(remoteArray)});
  }

  /// Roll back a failed optimistic create (TS `rollbackCreate`).
  Future<void> rollbackCreate(RecordId recordId, String tableName) async {
    final id = encodeRecordId(recordId);
    _local.delete(id);
    await _cache.delete(tableName, id, skipDbDelete: true);
    _removeRecordFromQueries(recordId);
  }

  /// Roll back a failed optimistic update by restoring [beforeRecord]
  /// (TS `rollbackUpdate`).
  Future<void> rollbackUpdate(
    RecordId recordId,
    String tableName,
    Map<String, dynamic> beforeRecord,
  ) async {
    final id = encodeRecordId(recordId);
    final content = {...beforeRecord}..remove('id');
    _local.replace(id, content);
    await _cache.save(
      CacheRecord(
        table: tableName,
        op: 'UPDATE',
        record: beforeRecord,
        version: ((beforeRecord['_00_rv'] as num?) ?? 1).toInt(),
      ),
      skipDbInsert: true,
    );
    _replaceRecordInQueries(beforeRecord);
  }

  void _removeRecordFromQueries(RecordId recordId) {
    final encodedId = encodeRecordId(recordId);
    for (final entry in _activeQueries.entries) {
      final records = entry.value.records;
      final index = records.indexWhere((r) {
        final rid = r['id'] is RecordId
            ? encodeRecordId(r['id'] as RecordId)
            : r['id'].toString();
        return rid == encodedId;
      });
      if (index != -1) {
        records.removeAt(index);
        _notify(entry.key, records);
      }
    }
  }

  /// Notify subscribers after a query's initial sync (TS `notifyQuerySynced`).
  /// Emits on first sync even with no results, so a `StreamBuilder` stops
  /// loading on an empty set.
  Future<void> notifyQuerySynced(String queryHash) async {
    final qs = _activeQueries[queryHash];
    if (qs == null) return;
    final newRecords = _materialize(qs.config.localArray);
    final changed = jsonEncode(qs.records) != jsonEncode(newRecords);
    qs.records = newRecords;
    if (changed || qs.updateCount == 0) {
      qs.updateCount++;
      _notify(queryHash, qs.records);
    }
  }

  // ==================== BACKEND RUN ====================

  /// Enqueue a backend job by writing an outbox record (TS `run`). Validates
  /// the route's declared args, then reuses [create] (local write + sync).
  Future<void> run(
    String backend,
    String path,
    Map<String, dynamic> data, {
    RunOptions? options,
  }) async {
    final backends = _schema['backends'];
    final backendDef = (backends is Map ? backends[backend] : null) as Map?;
    if (backendDef == null) throw ArgumentError('Backend $backend not found');
    final route = (backendDef['routes'] as Map?)?[path] as Map?;
    if (route == null) throw ArgumentError('Route $backend.$path not found');
    final tableName = backendDef['outboxTable'] as String?;
    if (tableName == null) {
      throw ArgumentError('Outbox table for backend $backend not found');
    }

    final args = (route['args'] as Map?) ?? const {};
    final payload = <String, dynamic>{};
    args.forEach((argName, argDef) {
      final optional = argDef is Map && argDef['optional'] == true;
      if (!data.containsKey(argName) && !optional) {
        throw ArgumentError('Missing required argument $argName');
      }
      payload[argName as String] = data[argName];
    });

    final record = <String, dynamic>{
      'path': path,
      'payload': jsonEncode(payload),
      'max_retries': options?.maxRetries ?? 3,
      'retry_strategy': options?.retryStrategy ?? 'linear',
    };
    if (options?.timeout != null) record['timeout'] = options!.timeout;
    if (options?.assignedTo != null)
      record['assigned_to'] = options!.assignedTo;

    await create('$tableName:${generateId()}', record);
  }

  // ==================== MUTATIONS ====================

  /// Create a record (TS `create`). Optimistic local write + DBSP ingest +
  /// mutation emission.
  Future<Map<String, dynamic>> create(
      String id, Map<String, dynamic> data) async {
    final tableName = extractTablePart(id);
    final columns = _columnsFor(tableName);
    final rid = parseRecordIdString(id);
    final params = parseParams(columns, data);
    final mutationId = RecordId('_00_pending_mutations', _nowStamp());

    final target = {...params, 'id': id, '_00_rv': 1};
    _local.tx(() {
      // Seed `_00_rv = 1` locally (the JS local schema defaults this column),
      // so a later UPDATE's `_00_rv += 1` reaches 2.
      _local.create(id, {...params, '_00_rv': 1});
      _local.putMutation(encodeRecordId(mutationId), {
        'mutationType': 'create',
        'recordId': id,
        'created_at': DateTime.now().toUtc().toIso8601String(),
      });
    });

    await _cache.save(
      CacheRecord(table: tableName, op: 'CREATE', record: target, version: 1),
      skipDbInsert: true,
    );

    final event = CreateEvent(
      mutationId: mutationId,
      recordId: rid,
      data: params,
      record: target,
      tableName: tableName,
    );
    _emitMutation([event]);
    return target;
  }

  /// Update a record (TS `update`).
  Future<Map<String, dynamic>> update(
    String table,
    String id,
    Map<String, dynamic> data, {
    UpdateOptions? options,
  }) async {
    final tableName = extractTablePart(id);
    final columns = _columnsFor(tableName);
    final rid = parseRecordIdString(id);
    final params = parseParams(columns, data);
    final mutationId = RecordId('_00_pending_mutations', _nowStamp());

    final beforeRecord = _local.getById(id);

    late Map<String, dynamic> target;
    _local.tx(() {
      _local.incrementRv(id);
      _local.updateMerge(id, params);
      _local.putMutation(encodeRecordId(mutationId), {
        'mutationType': 'update',
        'recordId': id,
        'data': _jsonSafe(params),
        'created_at': DateTime.now().toUtc().toIso8601String(),
      });
      target = _local.getById(id) ?? {'id': id, ...params};
    });

    // Build a partial record with only the fields the user changed.
    final updatedFields = <String, dynamic>{'id': target['id']};
    for (final key in data.keys) {
      if (target.containsKey(key)) updatedFields[key] = target[key];
    }
    if (target.containsKey('_00_rv'))
      updatedFields['_00_rv'] = target['_00_rv'];
    _replaceRecordInQueries(updatedFields);

    await _cache.save(
      CacheRecord(
        table: table,
        op: 'UPDATE',
        record: target,
        version: ((target['_00_rv'] as num?) ?? 1).toInt(),
      ),
      skipDbInsert: true,
    );

    final event = UpdateEvent(
      mutationId: mutationId,
      recordId: rid,
      data: params,
      record: target,
      beforeRecord: beforeRecord,
      options: parseUpdateOptions(id, data, options),
    );
    _emitMutation([event]);
    return target;
  }

  /// Delete a record (TS `delete`).
  Future<void> delete(String table, String id) async {
    final tableName = extractTablePart(id);
    _columnsFor(tableName); // validate table exists
    final rid = parseRecordIdString(id);
    final mutationId = RecordId('_00_pending_mutations', _nowStamp());

    final beforeRecord = _local.getById(id) ?? <String, dynamic>{};

    _local.tx(() {
      _local.delete(id);
      _local.putMutation(encodeRecordId(mutationId), {
        'mutationType': 'delete',
        'recordId': id,
        'created_at': DateTime.now().toUtc().toIso8601String(),
      });
    });

    await _cache.delete(table, id,
        skipDbDelete: true, recordData: beforeRecord);

    // DBSP may not emit view updates for DELETE; notify queries on this table.
    for (final entry in _activeQueries.entries) {
      if (entry.value.config.tableName == tableName) {
        await notifyQuerySynced(entry.key);
      }
    }

    _emitMutation([DeleteEvent(mutationId: mutationId, recordId: rid)]);
  }

  // ==================== INTERNALS ====================

  Future<String> _createAndRegisterQuery(
    String hash,
    RecordId recordId,
    String surqlString,
    Map<String, dynamic> params,
    QueryTimeToLive ttl,
    String tableName,
  ) async {
    final queryState =
        _createNewQuery(recordId, surqlString, params, ttl, tableName);

    final localArray = _cache.registerQuery(QueryPlanConfig(
      queryHash: hash,
      surql: surqlString,
      params: params,
      ttl: ttl,
      lastActiveAt: DateTime.now(),
    ));

    queryState.config.localArray = localArray;
    queryState.records = _materialize(localArray);

    _local.patchQueryConfig(encodeRecordId(recordId), {
      'localArray': _encodeVersionArray(localArray),
      'rowCount': localArray.length,
    });

    _activeQueries[hash] = queryState;
    _startTTLHeartbeat(hash, queryState);
    return hash;
  }

  /// Self-rescheduling timer at 90% of the query's TTL (TS `startTTLHeartbeat`).
  /// Faithful to the JS lifecycle; additionally invokes [_onHeartbeat] so the
  /// client can keep the server-side registration alive.
  void _startTTLHeartbeat(String hash, QueryState queryState) {
    if (queryState.ttlTimer != null) return;
    final heartbeatTime = (queryState.ttlDurationMs * 0.9).floor();
    queryState.ttlTimer = Timer(Duration(milliseconds: heartbeatTime), () {
      queryState.ttlTimer = null;
      _onHeartbeat?.call(hash);
      _startTTLHeartbeat(hash, queryState);
    });
  }

  void _stopTTLHeartbeat(QueryState queryState) {
    queryState.ttlTimer?.cancel();
    queryState.ttlTimer = null;
  }

  /// Cancel all heartbeat and debounce timers (call on client shutdown).
  void dispose() {
    for (final qs in _activeQueries.values) {
      _stopTTLHeartbeat(qs);
    }
    for (final timer in _debounceTimers.values) {
      timer.cancel();
    }
    _debounceTimers.clear();
  }

  QueryState _createNewQuery(
    RecordId recordId,
    String surqlString,
    Map<String, dynamic> params,
    QueryTimeToLive ttl,
    String tableName,
  ) {
    _columnsFor(tableName); // validate table exists

    final id = encodeRecordId(recordId);
    var configRecord = _local.getQueryConfig(id);
    if (configRecord == null) {
      configRecord = {
        'surql': surqlString,
        'params': _jsonSafe(params),
        'localArray': <dynamic>[],
        'remoteArray': <dynamic>[],
        'lastActiveAt': DateTime.now().toUtc().toIso8601String(),
        'createdAt': DateTime.now().toUtc().toIso8601String(),
        'ttl': ttl,
        'tableName': tableName,
        'updateCount': 0,
        'rowCount': 0,
        'errorCount': 0,
      };
      _local.putQueryConfig(id, configRecord);
    }

    final config = QueryConfig(
      id: recordId,
      surql: surqlString,
      params: params,
      localArray: _decodeVersionArray(configRecord['localArray']),
      remoteArray: _decodeVersionArray(configRecord['remoteArray']),
      ttl: ttl,
      lastActiveAt: DateTime.now(),
      tableName: tableName,
    );

    return QueryState(
      config: config,
      records: [],
      ttlDurationMs: parseDuration(ttl),
      updateCount: ((configRecord['updateCount'] as num?) ?? 0).toInt(),
      errorCount: ((configRecord['errorCount'] as num?) ?? 0).toInt(),
    );
  }

  /// SHA-256 of `{...data, sessionId}` as lowercase hex (TS `calculateHash`).
  ///
  /// Key order matches the JS object literal (`surql`, `params`, then
  /// `sessionId`) so the hash agrees with the JS client for a shared
  /// `_00_query` table.
  String calculateHash(Map<String, dynamic> data) {
    // Sanitize RecordId/DateTime params (jsonEncode can't encode them) the same
    // way they serialize on the wire, so the hash is stable.
    final content =
        jsonEncode({..._jsonSafe(data) as Map, 'sessionId': _sessionId});
    return sha256.convert(utf8.encode(content)).toString();
  }

  void _replaceRecordInQueries(Map<String, dynamic> record) {
    for (final entry in _activeQueries.entries) {
      final records = entry.value.records;
      final index = records.indexWhere((r) => r['id'] == record['id']);
      if (index != -1) {
        records[index] = {...records[index], ...record};
        _notify(entry.key, records);
      }
    }
  }

  void _notify(String queryHash, List<Map<String, dynamic>> records) {
    final subs = _subscriptions[queryHash];
    if (subs == null) return;
    for (final callback in subs.toList()) {
      callback(records);
    }
  }

  void _emitMutation(List<UpEvent> events) {
    for (final callback in _mutationCallbacks.toList()) {
      callback(events);
    }
  }

  Map<String, ColumnSchema> _columnsFor(String tableName) {
    final table = _schema[tableName];
    if (table == null) {
      throw ArgumentError('Table $tableName not found');
    }
    final columns = (table is Map && table['columns'] is Map)
        ? table['columns'] as Map
        : (table as Map);
    return columns
        .map((key, value) => MapEntry(key as String, value as ColumnSchema));
  }

  String _nowStamp() => DateTime.now().microsecondsSinceEpoch.toString();

  /// Strip non-JSON values (e.g. [RecordId], [DateTime]) for storage.
  dynamic _jsonSafe(dynamic value) {
    if (value is RecordId) return value.encode();
    if (value is DateTime) return value.toUtc().toIso8601String();
    if (value is Map) {
      return value.map((k, v) => MapEntry(k.toString(), _jsonSafe(v)));
    }
    if (value is List) return value.map(_jsonSafe).toList();
    return value;
  }

  List<dynamic> _encodeVersionArray(RecordVersionArray array) =>
      array.map((e) => [e.$1, e.$2]).toList();

  RecordVersionArray _decodeVersionArray(dynamic raw) {
    if (raw is! List) return [];
    return raw
        .map<RecordVersion>(
            (e) => ((e as List)[0] as String, (e[1] as num).toInt()))
        .toList();
  }
}

/// Build push-event debounce options from [UpdateOptions] (TS `parseUpdateOptions`).
PushEventOptions parseUpdateOptions(
  String id,
  Map<String, dynamic> data,
  UpdateOptions? options,
) {
  final debounced = options?.debounced;
  if (debounced == null || debounced == false) {
    return const PushEventOptions();
  }
  final opts = debounced is DebounceOptions ? debounced : null;
  final delay = opts?.delay ?? 200;
  final isFieldsKey = opts?.key == DebounceKey.recordIdXFields;
  final key =
      isFieldsKey ? '$id::${(data.keys.toList()..sort()).join('#')}' : id;
  return PushEventOptions(debounced: DebouncedConfig(key: key, delay: delay));
}
