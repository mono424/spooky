import 'dart:async';

import '../../events/event_system.dart';
import '../../services/database/local_database_service.dart';
import '../../services/database/remote_database_service.dart';
import '../../services/logger/logger.dart';
import '../../surreal/remote_client.dart';
import '../../surreal/value.dart';
import '../../types.dart';
import '../../utils/parser.dart';
import '../../utils/record_id_utils.dart';
import '../../utils/surql.dart';
import '../cache/cache_module.dart';
import '../data/data_module.dart';
import '../ref_tables.dart';
import 'engine.dart';
import 'queue/queue_down.dart';
import 'queue/queue_up.dart';
import 'scheduler.dart';
import 'sync_events.dart';
import 'sync_utils.dart';

/// Tunables for [Sp00kySync].
class Sp00kySyncOptions {
  const Sp00kySyncOptions({this.refSyncIntervalMs});
  final int? refSyncIntervalMs;
}

/// Bidirectional sync engine (TS `Sp00kySync`): up-queue (local -> remote),
/// down-queue (query registration/sync), a LIVE subscription on
/// `_00_list_ref[_user_<id>]`, and a self-rescheduling poll fallback.
class Sp00kySync {
  Sp00kySync(
    this._local,
    this._remote,
    this._cache,
    this._dataModule,
    this._schema,
    SpookyLogger logger, {
    Sp00kySyncOptions? options,
  }) : _logger = logger.child('Sp00kySync') {
    _upQueue = UpQueue(_local, _logger);
    _downQueue = DownQueue(_logger);
    _syncEngine = SyncEngine(_remote, _cache, _schema, _logger);
    _scheduler = SyncScheduler(
      _upQueue,
      _downQueue,
      _processUpEvent,
      _processDownEvent,
      _logger,
      onRollback: _handleRollback,
    );
    refSyncIntervalMs = resolveListRefPollInterval(options?.refSyncIntervalMs);
  }

  final LocalDatabaseService _local;
  final RemoteDatabaseService _remote;
  final CacheModule _cache;
  final DataModule _dataModule;
  final Map<String, dynamic> _schema;
  final SpookyLogger _logger;

  late final UpQueue _upQueue;
  late final DownQueue _downQueue;
  late final SyncEngine _syncEngine;
  late final SyncScheduler _scheduler;
  late final int refSyncIntervalMs;

  final EventSystem events = createSyncEventSystem();
  EventSystem get engineEvents => _syncEngine.events;

  bool _isInit = false;
  bool _closed = false;
  bool _wasDisconnected = false;
  final RefMode _refMode = defaultRefMode;

  String? _currentLiveQueryId;
  StreamSubscription<LiveMessage>? _liveSub;

  Timer? _listRefPollTimer;
  bool _listRefPollRunning = false;
  int? _lastLiveEventAt;

  int _liveRetryCount = 0;
  int get liveRetryCount => _liveRetryCount;

  int get pendingMutationCount => _upQueue.size;
  bool get isSyncing => _scheduler.isSyncing;

  void Function() subscribeToPendingMutations(void Function(int count) cb) {
    final id1 = _upQueue.events.subscribe(SyncQueueEventTypes.mutationEnqueued,
        (e) => cb((e.payload as Map)['queueSize'] as int));
    final id2 = _upQueue.events.subscribe(SyncQueueEventTypes.mutationDequeued,
        (e) => cb((e.payload as Map)['queueSize'] as int));
    return () {
      _upQueue.events.unsubscribe(id1);
      _upQueue.events.unsubscribe(id2);
    };
  }

  Future<void> init() async {
    if (_isInit) throw StateError('Sp00kySync is already initialized');
    _isInit = true;
    await _scheduler.init();
    _subscribeToReconnect();
    unawaited(_scheduler.syncUp());
    unawaited(_scheduler.syncDown());
    // No initial LIVE subscription: wait for setCurrentUserId from auth.
  }

  /// Push the authenticated user id; (re)starts the `_00_list_ref` LIVE under
  /// the new auth context with backoff, and the poll fallback. Null on sign-out.
  Future<void> setCurrentUserId(String? userId) async {
    _currentUserIdMirror ??= userId;
    if (_currentUserIdMirror == userId && _currentLiveQueryId != null) return;
    _currentUserIdMirror = userId;

    if (userId == null) {
      await _killRefLiveQuery();
      _stopListRefPoll();
      return;
    }

    _startListRefPoll();
    const attemptDelays = [0, 250, 500, 1000, 2000];
    for (var i = 0; i < attemptDelays.length; i++) {
      if (attemptDelays[i] > 0) {
        _liveRetryCount++;
        await Future<void>.delayed(Duration(milliseconds: attemptDelays[i]));
      }
      if (_closed) return; // bail if torn down mid-backoff
      try {
        await _restartRefLiveQuery();
        return;
      } catch (err) {
        _logger.debug(
            'Ref LIVE start failed (attempt ${i + 1}); poll fallback active');
      }
    }
  }

  String? _currentUserIdMirror;

  void enqueueDownEvent(DownEvent event) => _scheduler.enqueueDownEvent(event);
  Future<void> enqueueMutation(List<UpEvent> mutations) async =>
      _scheduler.enqueueMutation(mutations);

  /// Resolve the active `_00_list_ref` table name (TS `listRefTable`). Reads
  /// the user id from [DataModule] (set synchronously from the auth callback).
  String listRefTable() =>
      listRefTableFor(_refMode, _dataModule.getCurrentUserId());

  // ---- poll fallback --------------------------------------------------------

  /// Periodic re-poll of `_00_list_ref` as a safety net for missed LIVE
  /// notifications (TS `startListRefPoll`). SurrealDB v3 occasionally drops LIVE
  /// deliveries across sessions even when the row matches the permission rule,
  /// so the LIVE subscription alone can leave a query stale until reload; this
  /// catches those. Self-rescheduling (not a fixed interval) so each tick picks
  /// its own delay via [nextPollDelayMs] — an idle page coasts toward the slow
  /// cap and any activity snaps it back to the fast base cadence.
  void _startListRefPoll() {
    if (_listRefPollRunning) return;
    _listRefPollRunning = true;
    void schedule(int delayMs) {
      _listRefPollTimer = Timer(Duration(milliseconds: delayMs), () async {
        if (!_listRefPollRunning) return;
        try {
          await _pollListRefForActiveQueries();
        } finally {
          if (_listRefPollRunning) {
            schedule(nextPollDelayMs(
              now: DateTime.now().millisecondsSinceEpoch,
              lastLiveEventAt: _lastLiveEventAt,
              baseIntervalMs: refSyncIntervalMs,
            ));
          }
        }
      });
    }

    schedule(refSyncIntervalMs);
  }

  void _stopListRefPoll() {
    _listRefPollRunning = false;
    _listRefPollTimer?.cancel();
    _listRefPollTimer = null;
  }

  Future<void> _pollListRefForActiveQueries() async {
    for (final hash in _dataModule.getActiveQueryHashes()) {
      try {
        await _refetchListRefForQuery(hash);
      } catch (err) {
        _logger.debug('Per-query list_ref poll failed for $hash: $err');
      }
    }
  }

  Future<void> _refetchListRefForQuery(String queryHash) async {
    final queryState = _dataModule.getQueryByHash(queryHash);
    if (queryState == null) return;
    final fresh = await _fetchListRef(queryState.config.id);
    await _dataModule.updateQueryRemoteArray(queryHash, fresh);
    try {
      await syncQuery(queryHash);
    } catch (err) {
      _logger.info('syncQuery failed during poll: $err');
    }
  }

  // ---- LIVE -----------------------------------------------------------------

  Future<void> _killRefLiveQuery() async {
    await _liveSub?.cancel();
    _liveSub = null;
    if (_currentLiveQueryId != null) {
      try {
        await _remote.getClient().kill(_currentLiveQueryId!);
      } catch (err) {
        _logger.debug('Prior LIVE KILL failed; continuing: $err');
      }
      _currentLiveQueryId = null;
    }
  }

  Future<void> _restartRefLiveQuery() async {
    await _killRefLiveQuery();
    await _startRefLiveQueries();
  }

  void _subscribeToReconnect() {
    final client = _remote.getClient();
    client.onDisconnected.listen((_) {
      _wasDisconnected = true;
      _logger.info('Remote disconnected');
    });
    client.onConnected.listen((_) {
      if (!_wasDisconnected) return;
      _wasDisconnected = false;
      _logger.info('Remote reconnected, refetching active queries');
      for (final hash in _dataModule.getActiveQueryHashes()) {
        _scheduler.enqueueDownEvent(RegisterEvent(hash));
      }
    });
  }

  Future<void> _startRefLiveQueries() async {
    final tableName = listRefTable();
    final (liveId, stream) =
        await _remote.getClient().live('LIVE SELECT * FROM $tableName');
    _currentLiveQueryId = liveId;
    _liveSub = stream.listen((message) {
      if (message.action == 'KILLED') return;
      final value = message.value;
      _handleRemoteListRefChange(
        message.action,
        parseRecordIdString(value['in'].toString()),
        parseRecordIdString(value['out'].toString()),
        ((value['version'] as num?) ?? 0).toInt(),
      ).catchError((Object err) {
        _logger.error('Error handling remote list ref change', err);
      });
    });
  }

  Future<void> _handleRemoteListRefChange(
    String action,
    RecordId queryId,
    RecordId recordId,
    int version,
  ) async {
    _lastLiveEventAt = DateTime.now().millisecondsSinceEpoch;

    final existing = _dataModule.getQueryById(queryId);
    if (existing == null) {
      _logger.warn('Received remote update for unknown local query $queryId');
      return;
    }

    // DELETE is handled like CREATE/UPDATE (mirrors the TS core). When a record
    // leaves a query's window — e.g. it's deleted on the website or another
    // device — the SSP removes its `_00_list_ref` edge and LIVE delivers a
    // DELETE here; turn that into a `removed` diff so the row drops in realtime.
    // (Previously this returned early, so removals only caught up on the slow
    // poll — i.e. realtime deletion appeared not to work.) A delete
    // notification's `version` isn't reliable, so bypass the version guard;
    // SyncEngine re-verifies the record is gone upstream before deleting
    // locally, so a forced removal is safe.
    final diff = action == 'DELETE'
        ? RecordVersionDiff(added: [], updated: [], removed: [recordId])
        : createDiffFromDbOp(
            action, recordId, version, existing.config.localArray);
    await _syncRecordsForQuery(extractIdPart(queryId), diff);
  }

  // ---- up/down processing ---------------------------------------------------

  Future<void> _processUpEvent(UpEvent event) async {
    switch (event) {
      case CreateEvent(:final recordId, :final data):
        final setItems =
            data.keys.map((k) => SetItem.keyVar(k, 'data_$k')).toList();
        final vars = <String, dynamic>{'id': recordId};
        for (final k in data.keys) {
          vars['data_$k'] = data[k];
        }
        await _remote.query(surql.seal(surql.createSet('id', setItems)), vars);
      case UpdateEvent(:final recordId, :final data):
        await _remote
            .query('UPDATE \$id MERGE \$data', {'id': recordId, 'data': data});
      case DeleteEvent(:final recordId):
        await _remote.query('DELETE \$id', {'id': recordId});
    }
  }

  Future<void> _handleRollback(UpEvent event, Object error) async {
    final recordIdStr = encodeRecordId(event.recordId);
    final tableName = event is CreateEvent && event.tableName != null
        ? event.tableName!
        : extractTablePart(recordIdStr);

    switch (event) {
      case CreateEvent():
        await _dataModule.rollbackCreate(event.recordId, tableName);
      case UpdateEvent(:final beforeRecord):
        if (beforeRecord != null) {
          await _dataModule.rollbackUpdate(
              event.recordId, tableName, beforeRecord);
        } else {
          _logger.warn(
              'Cannot rollback update: no beforeRecord. Down-sync will reconcile.');
        }
      case DeleteEvent():
        _logger
            .warn('Delete rollback not implemented. Down-sync will reconcile.');
    }

    events.emit(SyncEventTypes.mutationRolledBack, {
      'eventType': event.runtimeType.toString(),
      'recordId': recordIdStr,
      'error': error.toString(),
    });
  }

  Future<void> _processDownEvent(DownEvent event) async {
    switch (event) {
      case RegisterEvent(:final hash):
        return _registerQuery(hash);
      case SyncDownEvent(:final hash):
        return syncQuery(hash);
      case HeartbeatEvent(:final hash):
        return _heartbeatQuery(hash);
      case CleanupEvent(:final hash):
        return _cleanupQuery(hash);
    }
  }

  Future<void> syncQuery(String hash) async {
    final queryState = _dataModule.getQueryByHash(hash);
    if (queryState == null) return;
    final diff =
        ArraySyncer(queryState.config.localArray, queryState.config.remoteArray)
            .nextSet();
    if (diff == null) return;
    return _syncRecordsForQuery(hash, diff);
  }

  /// Fetch the diff's record bodies via [SyncEngine], flipping the query's
  /// fetch status to `fetching` while bodies are pulled and back to `idle` when
  /// done (TS `Sp00kySync.syncRecords`). Status only flips when there is
  /// something to fetch (adds/updates); a pure-removal diff stays idle.
  Future<void> _syncRecordsForQuery(String hash, RecordVersionDiff diff) async {
    final fetching = diff.added.isNotEmpty || diff.updated.isNotEmpty;
    if (fetching) _dataModule.setQueryStatus(hash, QueryStatus.fetching);
    try {
      await _syncEngine.syncRecords(diff);
    } finally {
      if (fetching) _dataModule.setQueryStatus(hash, QueryStatus.idle);
    }
  }

  Future<void> _registerQuery(String queryHash) async {
    await _createRemoteQuery(queryHash);
    await syncQuery(queryHash);
    await _dataModule.notifyQuerySynced(queryHash);
  }

  Future<void> _createRemoteQuery(String queryHash) async {
    final queryState = _dataModule.getQueryByHash(queryHash);
    if (queryState == null) throw StateError('Query to register not found');

    await _remote.query('fn::query::register(\$config)', {
      'config': {
        'id': queryState.config.id,
        'surql': queryState.config.surql,
        'params': queryState.config.params,
        'ttl': queryState.config.ttl,
      },
    });

    final array = await _fetchListRef(queryState.config.id);
    await _dataModule.updateQueryRemoteArray(queryHash, array);
  }

  Future<RecordVersionArray> _fetchListRef(RecordId queryId) async {
    final results = await _remote
        .query(buildListRefSelect(listRefTable()), {'in': queryId});
    final items = firstRows(results);
    return items
        .map<({String out, int version})>((item) {
          final m = item as Map;
          return (
            out: m['out'].toString(),
            version: ((m['version'] as num?) ?? 0).toInt()
          );
        })
        .map<(String, int)>((e) => (e.out, e.version))
        .toList();
  }

  Future<void> _heartbeatQuery(String queryHash) async {
    final queryState = _dataModule.getQueryByHash(queryHash);
    if (queryState == null) throw StateError('Query to register not found');
    await _remote
        .query('fn::query::heartbeat(\$id)', {'id': queryState.config.id});
  }

  Future<void> _cleanupQuery(String queryHash) async {
    final queryState = _dataModule.getQueryByHash(queryHash);
    if (queryState == null) throw StateError('Query to register not found');
    await _remote.query('DELETE \$id', {'id': queryState.config.id});
    // Free the local DBSP view + in-memory state now that the remote `_00_query`
    // row is gone (TS `cleanupQuery` -> `finalizeDeregister`).
    _dataModule.finalizeDeregister(queryHash);
  }

  Future<void> close() async {
    _closed = true;
    _stopListRefPoll();
    await _killRefLiveQuery();
  }
}
