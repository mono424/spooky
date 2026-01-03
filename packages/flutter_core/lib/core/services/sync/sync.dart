import 'dart:async';
import '../database/local.dart';
import '../database/remote.dart';
import '../query/event.dart'; // QueryEvent and types
import '../events/main.dart'; // EventSystem
import '../../types.dart'; // IdTree, IdTreeDiff

import 'queue/queue_up.dart';
import 'queue/queue_down.dart';
import 'events.dart';
import 'utils.dart';
import '../query/utils.dart'; // extractResult

class SpookySync {
  late final UpQueue upQueue;
  late final DownQueue downQueue;

  bool _isInit = false;
  bool _isSyncingUp = false;
  bool _isSyncingDown = false;

  final LocalDatabaseService local;
  final RemoteDatabaseService remote;
  final EventSystem<dynamic> mutationEvents; // dynamic or MutationEvent
  final EventSystem<QueryEvent> queryEvents;

  bool get isSyncing => _isSyncingUp || _isSyncingDown;

  SpookySync({
    required this.local,
    required this.remote,
    required this.mutationEvents,
    required this.queryEvents,
  }) {
    upQueue = UpQueue(local);
    downQueue = DownQueue(local);
  }

  Future<void> init() async {
    print('syncing down');
    if (_isInit) throw Exception('SpookySync is already initialized');
    _isInit = true;

    await _initUpQueue();
    await _initDownQueue();

    // Fire and forget
    unawaited(_syncUp());
    unawaited(_syncDown());
  }

  Future<void> _initUpQueue() async {
    await upQueue.loadFromDatabase();
    // Subscribe to internal UP queue events to trigger sync
    upQueue.events.subscribe<MutationEnqueued>((e) => _syncUp());
    upQueue.listenForMutations(mutationEvents);
  }

  Future<void> _initDownQueue() async {
    // Listen to DOWN queue events to trigger sync
    downQueue.events.subscribe<SyncQueueEvent>((e) {
      if (e is IncantationRegistrationEnqueued ||
          e is IncantationSyncEnqueued ||
          e is IncantationCleanupEnqueued) {
        _syncDown();
      }
    });

    downQueue.listenForQueries(queryEvents);
  }

  Future<void> _syncUp() async {
    if (_isSyncingUp) return;
    _isSyncingUp = true;
    try {
      while (upQueue.size > 0) {
        await upQueue.next(_processUpEvent);
      }
    } finally {
      _isSyncingUp = false;
      unawaited(_syncDown());
    }
  }

  Future<void> _syncDown() async {
    if (_isSyncingDown) return;
    // Prioritize UpSync
    if (upQueue.size > 0) return;

    _isSyncingDown = true;
    try {
      while (downQueue.size > 0) {
        if (upQueue.size > 0) break;
        await downQueue.next(_processDownEvent);
      }
    } finally {
      _isSyncingDown = false;
    }
  }

  Future<void> _processUpEvent(UpEvent event) async {
    dynamic res;
    if (event is CreateEvent) {
      res = await remote.query(
        r'CREATE type::record($id) CONTENT $data',
        vars: {'id': event.recordId, 'data': event.data},
      );
    } else if (event is UpdateEvent) {
      res = await remote.query(
        r'UPDATE type::record($id) MERGE $data',
        vars: {'id': event.recordId, 'data': event.data},
      );
    } else if (event is DeleteEvent) {
      res = await remote.query(
        r'DELETE type::record($id)',
        vars: {'id': event.recordId},
      );
    }
    if (res.toString().contains("ERR") || res.toString().contains("error")) {
      throw Exception("Remote sync failed: $res");
    }
  }

  Future<void> _processDownEvent(DownEvent event) async {
    print('down event ${event.runtimeType}');
    if (event is RegisterEvent) {
      await _registerIncantation(event);
    } else if (event is SyncEvent) {
      await _syncIncantation(event);
    } else if (event is HeartbeatEvent) {
      await _heartbeatIncantation(event);
    } else if (event is CleanupEvent) {
      await _cleanupIncantation(event);
    }
  }

  Future<void> _registerIncantation(RegisterEvent event) async {
    final payload = event.payload;
    try {
      await _updateLocalIncantation(
        payload.incantationId,
        surrealql: payload.surrealql,
        hash: '', // Initial empty
        tree: null,
        params: payload.params,
      );

      await remote.query(
        r'UPSERT $id CONTENT $content',
        vars: {
          'id': payload.incantationId,
          'content': {
            'surrealql': payload.surrealql,
            'params': payload.params,
            'ttl': payload.ttl.value,
          },
        },
      );
    } catch (e) {
      print('[SpookySync] registerIncantation error: $e');
      rethrow;
    }
  }

  Future<void> _syncIncantation(SyncEvent event) async {
    final payload = event.payload;
    // TS: const { incantationId, surrealql, localTree, localHash, remoteHash, remoteTree } = event.payload;
    // NOTE: SyncEvent payload only has minimal info?
    // In TS: IncantationSyncEnqueued payload has { incantationId, remoteHash }
    // But handleIncomingRemoteUpdate in QueryManager provides more.
    // Wait, the flow is:
    // QueryManager emits IncantationRemoteHashUpdate -> DownQueue -> SyncEvent.
    // IncantationRemoteHashUpdatePayload has ALL fields (localHash, remoteHash, etc).
    // So SyncEvent IS carrying full payload.

    final isDifferent = payload.localHash != payload.remoteHash;
    if (!isDifferent) return;

    await _cacheMissingRecords(
      payload.localTree,
      payload.remoteTree,
      payload.surrealql,
    );

    await _updateLocalIncantation(
      payload.incantationId,
      surrealql: payload.surrealql,
      hash: payload.remoteHash,
      tree: payload.remoteTree,
      // params not in payload but likely active query handles it or ignored for cachedResults
    );
  }

  Future<IdTreeDiff> _cacheMissingRecords(
    dynamic localTreeJson,
    dynamic remoteTreeJson,
    String surrealql,
  ) async {
    // Parse JSON trees to IdTree objects if needed
    // Assuming they are passed as Maps/Lists suitable for IDTree.fromJson
    // If null, pass null.
    final localTree = localTreeJson != null
        ? IdTree.fromJson(localTreeJson)
        : null;
    final remoteTree = remoteTreeJson != null
        ? IdTree.fromJson(remoteTreeJson)
        : null;

    if (localTree == null) {
      // Initial fetch
      // TS: this.remote.getClient().query(surrealql).collect()
      // RemoteDatabaseService wrapper query returns JSON string of result?
      // TODO: RemoteDatabaseService abstraction of query might be limiting if we need raw array.
      // Assuming standard query behavior.

      // Note: using direct client if possible or parsing result string.
      // RemoteDatabaseService currently returns Future<String>.
      final resStr = await remote.query(surrealql);
      // Parse resStr -> List<Map>
      // Assuming extractResult or direct decode
      final dynamic raw = extractResult(resStr);
      List<Map<String, dynamic>> records = [];

      if (raw is List) {
        records = raw.cast<Map<String, dynamic>>();
      }

      await _cacheResults(records);
      return IdTreeDiff(added: records.map((r) => r['id'] as String).toList());
    }

    final diff = diffIdTree(localTree, remoteTree);
    final idsToFetch = [...diff.added, ...diff.updated];

    if (idsToFetch.isEmpty) {
      return IdTreeDiff();
    }

    // Fetch missing
    // TS: SELECT * FROM $ids
    final resStr = await remote.query(
      r'SELECT * FROM $ids',
      vars: {'ids': idsToFetch},
    );
    final dynamic raw = extractResult(resStr);
    List<Map<String, dynamic>> records = [];
    if (raw is List) {
      records = raw.cast<Map<String, dynamic>>();
    }

    await _cacheResults(records);
    return IdTreeDiff(added: records.map((r) => r['id'] as String).toList());
  }

  Future<void> _updateLocalIncantation(
    String incantationId, {
    required String surrealql,
    required String hash,
    dynamic tree,
    Map<String, dynamic>? params,
  }) async {
    await _updateIncantationRecord(incantationId, hash: hash, tree: tree);

    try {
      // Query local to get fresh data for UI
      final resStr = await local.query(surrealql, vars: params);
      final dynamic raw = extractResult(resStr);
      List<Map<String, dynamic>> cachedResults = [];
      if (raw is List) {
        cachedResults = raw.cast<Map<String, dynamic>>();
      }

      queryEvents.addEvent(
        IncantationIncomingRemoteUpdate(
          IncantationIncomingRemoteUpdatePayload(
            incantationId: incantationId,
            remoteHash: hash,
            remoteTree: tree,
            records: cachedResults,
          ),
        ),
      );
    } catch (e) {
      print('[SpookySync] failed to query local db or emit event: $e');
    }
  }

  Future<void> _updateIncantationRecord(
    String incantationId, {
    required String hash,
    dynamic tree,
  }) async {
    await local.query(
      r'UPDATE $id MERGE $content',
      vars: {
        'id': incantationId,
        'content': {'hash': hash, 'tree': tree},
      },
    );
  }

  Future<void> _cacheResults(List<Map<String, dynamic>> results) async {
    if (results.isEmpty) return;

    // TS uses transaction. Dart engine might not expose tx helper yet?
    // We can just loop upserts or batch.
    // BEGIN TRANSACTION;
    // ... UPSERT ...
    // COMMIT TRANSACTION;

    // Construct massive query string? Or iterate?
    // Iterating is safer for now. optimize later.

    // Actually, local DB is fast.
    // Use transaction for consistency.

    // Better: pass entire array as parameter and use FOR loop if possible.
    // engine supports JSON vars.

    final query = r'''
      BEGIN TRANSACTION;
      FOR $record IN $records {
        UPSERT $record.id CONTENT $record;
      };
      COMMIT TRANSACTION;
    ''';

    await local.query(query, vars: {'records': results});
  }

  Future<void> _heartbeatIncantation(HeartbeatEvent event) async {
    await remote.query(
      r'fn::incantation::heartbeat($id)',
      vars: {'id': event.payload.incantationId},
    );
  }

  Future<void> _cleanupIncantation(CleanupEvent event) async {
    await remote.query(
      r'DELETE $id',
      vars: {'id': event.payload.incantationId},
    );
  }
}
