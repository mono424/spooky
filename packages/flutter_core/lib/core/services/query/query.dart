import 'dart:async';
import 'dart:convert';

import '../database/local.dart';
import '../database/remote.dart';
import '../../types.dart' hide Incantation;
import '../events/main.dart'; // EventSystem
import '../sync/utils.dart'; // flattenIdTree

import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

import 'event.dart';
import 'incantation.dart';
import 'utils.dart'; // extractResult, parseRecordIdString
import '../mutation/events.dart'; // MutationEvent

class QueryManager {
  // Active queries map: hash -> Incantation
  final Map<String, Incantation<dynamic>> activeQueries = {};

  // Custom Event System for Query Events
  late final EventSystem<QueryEvent> events;

  final LocalDatabaseService local;
  final RemoteDatabaseService remote;
  final String? clientId;
  final bool enableLiveQuery;
  final EventSystem<dynamic> mutationEvents;

  // Constructor
  QueryManager({
    required this.local,
    required this.remote,
    required this.mutationEvents,
    this.clientId,
    this.enableLiveQuery = true,
  }) {
    events = EventSystem<QueryEvent>();
    // Subscribe to incoming remote updates
    events.subscribe<IncantationIncomingRemoteUpdate>(
      _handleIncomingRemoteUpdate,
    );
    // Subscribe to local mutations
    mutationEvents.subscribe<MutationEvent>(_handleLocalMutation);
  }

  // --- Public Methods ---

  Future<void> init() async {
    if (enableLiveQuery) {
      await _startLiveQuery();
    }
  }

  /// Register a new query/incantation.
  /// Returns the Hash ID of the query.
  Future<String> register({
    required String tableName,
    required String surrealql,
    required Map<String, dynamic> params,
    required QueryTimeToLive ttl,
  }) async {
    final id = await _calculateHash(surrealql, params);

    // Persist to local _spooky_incantation table
    final recordId = '_spooky_incantation:$id';

    // Prepare content
    final content = {
      'Id': id,
      'SurrealQL': surrealql,
      'Params': params,
      'ClientId': clientId,
      'Hash': id,
      'Tree': null,
      'LastActiveAt': DateTime.now().toIso8601String(),
      'TTL': ttl.value,
    };

    await local.query(
      'UPSERT $recordId CONTENT \$data',
      vars: {'data': content},
    );

    // Add to active memory map
    if (!activeQueries.containsKey(id)) {
      final incantation = Incantation(
        id: recordId,
        surrealql: surrealql,
        hash: id,
        ttl: ttl,
        meta: IncantationMeta(tableName: tableName),
        params: params,
        tree: null,
        lastActiveAt: DateTime.now(),
      );

      activeQueries[id] = incantation;
      await _initLifecycle(incantation);
    }

    return id;
  }

  /// Alias for register
  Future<String> query({
    required String tableName,
    required String surrealql,
    required Map<String, dynamic> params,
    required QueryTimeToLive ttl,
  }) {
    return register(
      tableName: tableName,
      surrealql: surrealql,
      params: params,
      ttl: ttl,
    );
  }

  /// Subscribe to changes for a specific query hash.
  /// Returns a cancel function.
  void Function() subscribe(
    String queryHash,
    void Function(List<Map<String, dynamic>> records) callback, {
    bool immediate = false,
  }) {
    final subscription = events.subscribe<IncantationUpdated>((event) {
      if (event.payload.incantationId == '_spooky_incantation:$queryHash') {
        callback(event.payload.records);
      }
    });

    if (immediate) {
      final incantation = activeQueries[queryHash];
      if (incantation != null && incantation.records != null) {
        final records = incantation.records!.cast<Map<String, dynamic>>();
        callback(records);
      } else {
        callback([]);
      }
    }

    return () => subscription.cancel();
  }

  // --- Internals ---

  Future<void> _initLifecycle(Incantation incantation) async {
    events.addEvent(
      IncantationInitialized(
        IncantationInitializedPayload(
          incantationId: incantation.id,
          surrealql: incantation.surrealql,
          ttl: incantation.ttl,
          params: incantation.params,
        ),
      ),
    );

    incantation.startTTLHeartbeat(() {
      events.addEvent(
        IncantationTTLHeartbeat(
          IncantationTTLHeartbeatPayload(incantationId: incantation.id),
        ),
      );
    });
  }

  void _handleIncomingRemoteUpdate(IncantationIncomingRemoteUpdate event) {
    final payload = event.payload;
    final parts = payload.incantationId.split(':');
    final queryHash = parts.length > 1 ? parts[1] : parts[0];

    final incantation = activeQueries[queryHash];
    if (incantation == null) return;

    final records = payload.records;

    IdTree? remoteTreeObj;
    if (payload.remoteTree is IdTree) {
      remoteTreeObj = payload.remoteTree as IdTree;
    } else if (payload.remoteTree != null) {
      try {
        remoteTreeObj = IdTree.fromJson(payload.remoteTree);
      } catch (e) {}
    }

    final remoteLeaves = flattenIdTree(remoteTreeObj);
    final remoteIds = remoteLeaves.map((l) => l.id).toSet();

    final validRecords = <Map<String, dynamic>>[];
    final orphanedRecords = <Map<String, dynamic>>[];

    for (final record in records) {
      final id = record['id'];
      final idStr = id.toString();

      if (remoteIds.contains(idStr)) {
        validRecords.add(record);
      } else {
        orphanedRecords.add(record);
      }
    }

    incantation.updateLocalState(
      validRecords,
      payload.remoteHash,
      payload.remoteTree,
    );

    events.addEvent(
      IncantationUpdated(
        IncantationUpdatedPayload(
          incantationId: payload.incantationId,
          records: validRecords,
        ),
      ),
    );

    if (orphanedRecords.isNotEmpty) {
      _verifyAndPurgeOrphans(orphanedRecords);
    }
  }

  Future<void> _handleLocalMutation(MutationEvent event) async {
    final affectedTables = <String>{};
    for (final mutation in event.payload) {
      final id = mutation.record_id;
      if (id != null) {
        final parts = id.split(':');
        if (parts.isNotEmpty) affectedTables.add(parts[0]);
      }
    }

    if (affectedTables.isEmpty) return;

    for (final incantation in activeQueries.values) {
      final table = incantation.meta?.tableName;
      if (table != null && affectedTables.contains(table)) {
        await _refreshLocalIncantation(incantation);
      }
    }
  }

  Future<void> _refreshLocalIncantation(Incantation incantation) async {
    try {
      final resStr = await local.query(
        incantation.surrealql,
        vars: incantation.params,
      );
      final dynamic raw = extractResult(resStr);
      List<Map<String, dynamic>> records = [];
      if (raw is List) {
        records = raw.cast<Map<String, dynamic>>();
      }

      incantation.records = records;

      events.addEvent(
        IncantationUpdated(
          IncantationUpdatedPayload(
            incantationId: incantation.id,
            records: records,
          ),
        ),
      );
    } catch (e) {
      print('[QueryManager] Failed to refresh local incantation: $e');
    }
  }

  Future<void> _verifyAndPurgeOrphans(
    List<Map<String, dynamic>> orphans,
  ) async {
    if (orphans.isEmpty) return;

    final idsToCheck = orphans.map((r) {
      final idRaw = r['id'];
      if (idRaw is RecordId) return idRaw;
      return parseRecordIdString(idRaw.toString());
    }).toList();

    if (idsToCheck.isEmpty) return;

    try {
      final resStr = await remote.query(
        r'SELECT id FROM $ids',
        vars: {'ids': idsToCheck},
      );
      final dynamic raw = extractResult(resStr);

      final existingIds = <String>{};
      if (raw is List) {
        for (final item in raw) {
          existingIds.add(item['id'].toString());
        }
      }

      final toDelete = <RecordId>[];
      for (final id in idsToCheck) {
        if (id is RecordId && !existingIds.contains(id.toString())) {
          toDelete.add(id);
        }
      }

      if (toDelete.isNotEmpty) {
        await local.query(r'DELETE $ids', vars: {'ids': toDelete});
        print('[QueryManager] Purged ${toDelete.length} orphaned records.');
      }
    } catch (e) {
      print('[QueryManager] Failed to purge orphans: $e');
    }
  }

  Future<void> _startLiveQuery() async {
    remote.subscribeLive(
      tableName: '_spooky_incantation',
      callback: (action, result) {
        if (action == 'UPDATE' || action == 'CREATE') {
          final id = result['id'] as String?;
          final hash = result['Hash'] as String?;
          final tree = result['Tree'];

          if (id == null || hash == null) return;

          final parts = id.split(':');
          final queryHash = parts.length > 1 ? parts[1] : parts[0];

          final incantation = activeQueries[queryHash];
          if (incantation == null) return;

          events.addEvent(
            IncantationRemoteHashUpdate(
              IncantationRemoteHashUpdatePayload(
                incantationId: id,
                surrealql: incantation.surrealql,
                localHash: incantation.hash,
                localTree: incantation.tree,
                remoteHash: hash,
                remoteTree: tree,
              ),
            ),
          );
        }
      },
    );
  }

  Future<String> _calculateHash(
    String surrealql,
    Map<String, dynamic> params,
  ) async {
    final content = jsonEncode({'surrealql': surrealql, 'params': params});

    try {
      final resultJson = await local.query(
        r'RETURN crypto::blake3($content)',
        vars: {'content': content},
      );

      final rawList = jsonDecode(resultJson);
      if (rawList is List && rawList.isNotEmpty) {
        final val = extractResult(resultJson);
        return val.toString();
      }
    } catch (e) {
      print(
        "[QueryManager] Hash calc error: $e. Using simple string hash fallback.",
      );
    }

    return content.hashCode.toString();
  }
}
