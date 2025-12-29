import 'dart:async';
import 'dart:convert';

import '../database/local.dart';
import '../database/remote.dart';
import '../../types.dart' hide Incantation;
import '../events/main.dart'; // EventSystem

import 'event.dart';
import 'incantation.dart';
import 'utils.dart'; // extractResult

class QueryManager {
  // Active queries map: hash -> Incantation
  final Map<String, Incantation<dynamic>> activeQueries = {};

  // Custom Event System for Query Events
  late final EventSystem<QueryEvent> events;

  final LocalDatabaseService local;
  final RemoteDatabaseService remote;
  final String? clientId;

  // Constructor
  QueryManager({required this.local, required this.remote, this.clientId}) {
    events = EventSystem<QueryEvent>();
    // Subscribe to incoming remote updates
    events.subscribe<IncantationIncomingRemoteUpdate>(
      _handleIncomingRemoteUpdate,
    );
  }

  // --- Public Methods ---

  Future<void> init() async {
    await _startLiveQuery();
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
    // In Dart we use String ID directly for now, assuming RecordId format usage.
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
      'TTL': ttl.value, // Store as String duration (e.g. '10m')
    };

    // Upsert into local DB
    // Assuming local.query supports upsert or we use a custom query
    await local.query(
      'UPSERT $recordId CONTENT \$data',
      vars: {'data': content},
    );

    // Add to active memory map
    if (!activeQueries.containsKey(id)) {
      final incantation = Incantation(
        id: recordId,
        surrealql: surrealql,
        hash: id, // Initial hash matches ID
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
    // Listen to IncantationUpdated events
    final subscription = events.subscribe<IncantationUpdated>((event) {
      // In Dart, Incantation ID might be full `table:id`, queryHash is just `id` part?
      // In _calculateHash we return a string. In register we make `_spooky_incantation:$id`.
      // The event payload incantationId matches `incantation.id`.
      if (event.payload.incantationId == '_spooky_incantation:$queryHash') {
        callback(event.payload.records);
      }
    });

    if (immediate) {
      final incantation = activeQueries[queryHash];
      if (incantation != null && incantation.records != null) {
        // Cast dynamic list to Map list if necessary
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
    // Extract ID (remove table prefix if stored with it)
    // incantationId is like `_spooky_incantation:hash`
    // We stored it in map using pure hash.
    final parts = payload.incantationId.split(':');
    final queryHash = parts.length > 1 ? parts[1] : parts[0];

    final incantation = activeQueries[queryHash];
    if (incantation == null) return;

    final records = payload.records;
    // In TS we used decodeFromSpooky here. We skip for now.

    incantation.updateLocalState(
      records,
      payload.remoteHash,
      payload.remoteTree,
    );

    events.addEvent(
      IncantationUpdated(
        IncantationUpdatedPayload(
          incantationId: payload.incantationId,
          records: records,
        ),
      ),
    );
  }

  Future<void> _startLiveQuery() async {
    // We listen to changes on _spooky_incantation on the Remote DB
    remote.subscribeLive(
      tableName: '_spooky_incantation',
      callback: (action, result) {
        if (action == 'UPDATE' || action == 'CREATE') {
          // Expected result structure: {id: ..., Hash: ..., Tree: ...}
          final id = result['id'] as String?;
          final hash = result['Hash'] as String?;
          final tree = result['Tree'];

          if (id == null || hash == null) return;

          // Parse ID to get hash key
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

  /// Calculates the hash of the query parameters/content.
  /// Uses local DB `crypto::blake3` if available, or just fallback (stub logic).
  /// In TS it calls `RETURN crypto::blake3($content)`.
  Future<String> _calculateHash(
    String surrealql,
    Map<String, dynamic> params,
  ) async {
    final content = jsonEncode({'surrealql': surrealql, 'params': params});

    try {
      // Execute query on local DB to get hash
      // The local DB might not have the crypto functions enabled/loaded?
      // But let's try assuming it mirrors the full surreal features.
      final resultJson = await local.query(
        r'RETURN crypto::blake3($content)',
        vars: {'content': content},
      );

      // Parse result: `["hash_string"]` or similar depending on extractResult usage
      // extractResult returns the inner value.
      // If the query returns just the string, extractResult might need handling.
      // `local.query` returns a String (JSON).

      final rawList = jsonDecode(resultJson);
      // Usually returns [ "hash..." ] if it's a RETURN value query
      if (rawList is List && rawList.isNotEmpty) {
        // Check if it wrapped in status/result object or pure value (depends on driver)
        // The Rust engine usually returns `[{ "status": "OK", "result": "hash" }]`
        // extractResult handles this.

        final val = extractResult(resultJson);
        return val.toString();
      }
    } catch (e) {
      print(
        "[QueryManager] Hash calc error: $e. Using simple string hash fallback.",
      );
    }

    // Fallback: Dart hashCode (not safe for cross-device sync but allows running)
    return content.hashCode.toString();
  }
}
