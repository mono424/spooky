import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_core/core/services/database/remote.dart';
import 'package:flutter_core/core/services/sync/sync.dart';
import 'package:flutter_core/core/services/mutation/events.dart';
import 'package:flutter_core/core/services/query/event.dart';
import 'package:flutter_core/core/services/events/main.dart';
import 'package:flutter_core/core/services/sync/events.dart';
import 'package:flutter_core/core/types.dart' hide RecordId;

void main() {
  test('SpookySync UpSync Integration Test', () async {
    // 1. Init Rust
    final dylibPath =
        '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    try {
      await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));
    } catch (_) {}

    // 2. Setup Local DB
    final localConfig = DatabaseConfig(
      path: ":memory:",
      namespace: "sync_local",
      database: "db",
    );
    final localService = await LocalDatabaseService.create(localConfig);
    await localService.init();

    // Create pending mutations table
    await localService.getClient.query(
      sql: "DEFINE TABLE _spooky_pending_mutations SCHEMALESS;",
    );
    // Cleanup
    try {
      await localService.getClient.query(
        sql: "REMOVE TABLE _spooky_pending_mutations;",
      );
      await localService.getClient.query(sql: "REMOVE TABLE user;");
    } catch (_) {}

    // 3. Setup Remote DB (Simulated)
    final remoteConfig = DatabaseConfig(
      path: "unused",
      namespace: "sync_remote",
      database: "db",
      endpoint: "mem://", // Dummy
    );
    // Create in-memory client for remote
    final remoteClient = await SurrealDb.connect(mode: StorageMode.memory());
    final remoteService = RemoteDatabaseService.createWithClient(
      remoteClient,
      remoteConfig,
    );
    await remoteClient.useDb(
      ns: "sync_remote",
      db: "db",
    ); // Init remote explicitly

    // Cleanup remote
    try {
      await remoteClient.query(sql: "REMOVE TABLE user;");
    } catch (_) {}

    // 4. Setup Events
    final mutationEvents = EventSystem<MutationEvent>();
    final queryEvents = EventSystem<QueryEvent>();

    // 5. Create SpookySync
    final spooky = SpookySync(
      local: localService,
      remote: remoteService,
      mutationEvents: mutationEvents,
      queryEvents: queryEvents,
    );

    // 6. Pre-populate Pending Mutation (Offline capability test)
    // Create a mutation in local DB directly
    await localService.getClient.query(
      sql: r'''
        CREATE _spooky_pending_mutations SET 
            mutationType = 'create', 
            recordId = 'user:1', 
            data = {name: 'Offline User'},
            created_at = time::now()
        ''',
    );

    // 7. Init Sync
    await spooky.init();

    // Wait for async sync up
    // Queue load happens in init, syncUp is fired unregistered.
    // We check periodically.
    bool found = false;
    for (int i = 0; i < 20; i++) {
      final res = await remoteClient.query(sql: "SELECT * FROM user:1");
      // Result is json string.
      if (res.contains("Offline User")) {
        found = true;
        break;
      }
      await Future.delayed(Duration(milliseconds: 100));
    }
    expect(found, true, reason: "Offline mutation should be synced to remote");

    // 8. Live Sync Test
    // 8. Live Sync Test
    // But wait! UpQueue logic requires DB entry for delete.
    // So we mimic MutationManager: Insert into DB AND emit event.

    // But wait! UpQueue logic:
    // listenForMutations -> _handleMutationPayload -> push(event).
    // Does it INSERT into DB?
    // MutationManager inserts into DB.
    // UpQueue assumes it IS in DB if it loads from DB?
    // No, listenForMutations pushes to queue memory AND queue processing removes from DB.
    // BUT _handleMutationPayload does NOT insert into DB.
    // MutationManager does.
    // If SpookySync relies on MutationManager having inserted it?
    // UpQueue.next -> _processUpEvent -> removeEventFromDatabase.
    // If it's not in DB, removeEventFromDatabase might fail or do nothing.
    // Crucially: Does UpQueue REQUIRE it to be in DB?
    // No, `next` passes `UpEvent` which has data.
    // `removeEventFromDatabase` tries to delete it.

    // So for this test, we mimic MutationManager: Insert into DB AND emit event.
    // 1. Insert
    await localService.getClient.query(
      sql: r'''
        CREATE type::record($id) SET 
            mutationType = 'create', 
            recordId = 'user:2', 
            data = {name: 'Live User'},
            created_at = time::now()
        ''',
      vars: jsonEncode({'id': '_spooky_pending_mutations:mut_2'}),
    );

    // 2. Emit Event
    mutationEvents.addEvent(
      MutationEvent([
        MutationPayload(
          type: MutationAction.create,
          record_id: 'user:2',
          mutation_id:
              '_spooky_pending_mutations:mut_2', // ID must match DB for delete to work later?
          data: {'name': 'Live User'},
        ),
      ]),
    );

    // Wait for sync
    found = false;
    for (int i = 0; i < 20; i++) {
      final res = await remoteClient.query(sql: "SELECT * FROM user:2");
      if (res.contains("Live User")) {
        found = true;
        break;
      }
      await Future.delayed(Duration(milliseconds: 100));
    }
    expect(found, true, reason: "Live mutation should be synced to remote");

    // Verify Local Queue DB is empty
    final localPending = await localService.getClient.query(
      sql: "SELECT * FROM _spooky_pending_mutations",
    );
    // Should be empty or contain []
    expect(localPending.contains("Offline User"), false);
    expect(localPending.contains("Live User"), false);

    await localService.close();
    // remoteClient.close(); // SurrealDb doesn't always have close exposed in Dart binding properly?
    // AbstractDatabaseService has close.
    await remoteService.close();
  });
}
