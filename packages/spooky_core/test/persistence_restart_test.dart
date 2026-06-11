import 'dart:io';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/sqlite_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// Regression guard for the signed-in-history "0 games after restart" bug.
///
/// The framework persists everything through ONE store: the stream-processor
/// circuit state and the auth token live in the same sqlite as the document
/// bodies (`SqlitePersistenceClient(_local)`). The app had split that — circuit
/// state persisted (SharedPreferences) while the document store was in-memory —
/// so after a restart the restored circuit reported a non-empty query view
/// (`localArray = N`) but `DataModule._materialize` (`_local.getById`) found no
/// bodies, emitting 0 rows. And because the diff (`localArray == remoteArray`)
/// was empty, sync never re-fetched to repopulate the store.
///
/// With a FILE-BACKED `_local`, the documents AND the circuit state survive a
/// restart together, so the persisted view membership always has backing rows.
/// This test reproduces a restart with a real on-disk store (no server) and
/// asserts both halves round-trip. It would FAIL with an in-memory store.
void main() {
  final logger = SpookyLogger.root('test');

  test('file-backed store: documents + circuit state survive a restart', () async {
    final dir = Directory.systemTemp.createTempSync('spooky_persist');
    addTearDown(() => dir.deleteSync(recursive: true));
    final path = '${dir.path}/spooky.db';

    QueryPlanConfig plan() => QueryPlanConfig(
          queryHash: 'qg',
          surql: r'SELECT * FROM game WHERE database = $db',
          // String param mirrors the sanitized form the local circuit receives
          // (a RecordId is stringified for the FFI), matching the row field.
          params: {'db': 'game_database:DBS_x'},
          ttl: '10m',
          lastActiveAt: DateTime.utc(2026),
        );

    // --- session 1: register a record-filtered query, ingest matching rows --
    final local1 = LocalDatabaseService.open(logger,
        store: StoreType.indexeddb, path: path)
      ..provision();
    final sp1 = StreamProcessorService(SqlitePersistenceClient(local1), logger);
    await sp1.init();
    sp1.seedPermissionsFromSchema(
        'DEFINE TABLE game PERMISSIONS FOR select WHERE true;');
    sp1.registerQueryPlan(plan());
    final cache1 = CacheModule(local1, sp1, (_) {}, logger);
    await cache1.saveBatch([
      for (var i = 0; i < 3; i++)
        CacheRecord(
          table: 'game',
          op: 'CREATE',
          record: {
            'id': 'game:g$i',
            'database': 'game_database:DBS_x',
            'white': 'W$i',
          },
          version: 1,
        ),
    ]);
    await sp1.saveState(); // persist the circuit (incl. rows + view) to disk
    await sp1.close();
    local1.close();

    // --- session 2: reopen the SAME file (simulated app restart) ------------
    final local2 = LocalDatabaseService.open(logger,
        store: StoreType.indexeddb, path: path)
      ..provision();
    final sp2 = StreamProcessorService(SqlitePersistenceClient(local2), logger);
    await sp2.init(); // loadState() restores the circuit from disk
    addTearDown(() async {
      await sp2.close();
      local2.close();
    });
    sp2.seedPermissionsFromSchema(
        'DEFINE TABLE game PERMISSIONS FOR select WHERE true;');

    // Re-registering (what the app does on every launch) returns the snapshot
    // computed against the RESTORED store: the view membership persisted.
    final restored = sp2.registerQueryPlan(plan());
    expect(restored, isNotNull);
    final ids = restored!.localArray.map((e) => e.$1).toList();
    expect(ids, hasLength(3),
        reason: 'circuit view membership must persist across restart');

    // The regression: every persisted member resolves to a document body on
    // disk WITHOUT re-fetching from the server. (An in-memory store would have
    // wiped these, leaving `localArray = 3` but materializing 0.)
    for (final id in ids) {
      final doc = local2.getById(id);
      expect(doc, isNotNull,
          reason: 'document bodies must persist across restart ($id)');
      expect(doc!['database'], 'game_database:DBS_x');
    }

    // The auth token + circuit state share the same file, so a session also
    // survives — what keeps the user signed in offline.
    final persistence2 = SqlitePersistenceClient(local2);
    await persistence2.set('probe', 'value');
    expect(await persistence2.get<String>('probe'), 'value');
  });
}
