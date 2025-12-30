import 'dart:async';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

import 'types.dart';
import 'services/database/local.dart';
import 'services/database/remote.dart';
import 'services/database/local_migration.dart';
import 'services/mutation/main.dart'; // MutationManager
import 'services/query/query.dart'; // QueryManager
import 'services/sync/sync.dart'; // SpookySync

export 'types.dart';
export 'services/database/local.dart';
export 'services/database/remote.dart';
export 'services/mutation/main.dart';
export 'services/query/query.dart';
export 'services/query/utils.dart'; // extractResult

class SpookyClient {
  final SpookyConfig config;
  final LocalDatabaseService local;
  final RemoteDatabaseService remote;
  final LocalMigration migrator;
  final MutationManager mutation;
  final QueryManager queryManager;
  final SpookySync sync;

  SpookyClient._(
    this.config,
    this.local,
    this.remote,
    this.migrator,
    this.mutation,
    this.queryManager,
    this.sync,
  );

  static bool _rustInitialized = false;

  static Future<SpookyClient> init(SpookyConfig config) async {
    if (!_rustInitialized) {
      await RustLib.init();
      _rustInitialized = true;
    }

    // 1. Initialize DB Services
    final local = await LocalDatabaseService.connect(config.database);
    await local.init();

    final remote = await RemoteDatabaseService.connect(config.database);
    await remote.init();

    // 2. Run Migrations (Provision Schema)
    final migrator = LocalMigration(local);
    await migrator.provision(config.schemaSurql);

    // 3. Initialize Managers
    final mutation = MutationManager(local);

    // clientId logic from TS: default to random UUID if missing (TODO: where to store/get?)
    // For now null is fine or generating one.
    final queryManager = QueryManager(
      local: local,
      remote: remote,
      clientId:
          "client_${DateTime.now().millisecondsSinceEpoch}", // Simple unique ID
    );
    await queryManager.init();

    // 4. Initialize Sync
    // MutationManager in Dart exposes `events` getter?
    // Yes, but need to check mutation/main.dart or mutation/mutation.dart content.
    // Assuming MutationManager has `events`.

    final sync = SpookySync(
      local: local,
      remote: remote,
      mutationEvents: mutation.getEvents,
      queryEvents: queryManager.events,
    );
    await sync.init();

    return SpookyClient._(
      config,
      local,
      remote,
      migrator,
      mutation,
      queryManager,
      sync,
    );
  }

  Future<void> close() async {
    await local.close();
    await remote.close();
    // Also dispose active listeners if needed
  }

  // --- Public API Delegates ---

  /// Execute a tracked Live Query (Incantation).
  /// [surrealql] should conform to Spooky Query Builder output.
  Future<String> query({
    required String tableName,
    required String surrealql,
    required Map<String, dynamic> params,
    QueryTimeToLive ttl = QueryTimeToLive.tenMinutes,
  }) {
    return queryManager.query(
      tableName: tableName,
      surrealql: surrealql,
      params: params,
      ttl: ttl,
    );
  }

  /// Subscribe to a registered Incantation by its Hash.
  /// Returns a disposer function.
  Future<void Function()> subscribe(
    String queryHash,
    void Function(List<Map<String, dynamic>> records) callback, {
    bool immediate = false,
  }) async {
    return queryManager.subscribe(queryHash, callback, immediate: immediate);
  }

  // Wrappers for Mutation Manager
  Future<Map<String, dynamic>?> create(String id, Map<String, dynamic> data) {
    return mutation.create(id, data);
  }

  Future<Map<String, dynamic>?> update(
    String table,
    String id,
    Map<String, dynamic> data,
  ) {
    return mutation.update(id, data);
  }

  Future<void> delete(String table, String id) {
    // We ignore table as id is sufficient for delete(rid)
    return mutation.delete(id);
  }

  // Auth delegates
  Future<void> authenticate(String token) => remote.authenticate(token);
  Future<void> deauthenticate() => remote.invalidate();
}
