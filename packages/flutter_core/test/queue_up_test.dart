import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_core/core/services/sync/queue/queue_up.dart';
import 'package:flutter_core/core/services/mutation/events.dart';
import 'package:flutter_core/core/services/mutation/main.dart'; // MutationManager
import 'package:flutter_core/core/types.dart' hide RecordId;

void main() {
  test('UpQueue functionality test', () async {
    final dylibPath =
        '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    try {
      await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));
    } catch (_) {}

    final config = DatabaseConfig(
      path: ":memory:",
      namespace: "queue_test",
      database: "queue_db",
    );

    final localService = await LocalDatabaseService.create(config);
    await localService.init();

    // Setup schema
    try {
      await localService.getClient.query(
        sql: "REMOVE TABLE _spooky_pending_mutations;",
      );
    } catch (_) {}
    try {
      await localService.getClient.query(sql: "REMOVE TABLE user;");
    } catch (_) {}
    await localService.getClient.query(
      sql: "DEFINE TABLE _spooky_pending_mutations SCHEMALESS;",
    );

    final queue = UpQueue(localService);
    final mutationManager = MutationManager(localService);

    // Wire up listeners
    queue.listenForMutations(mutationManager.getEvents);

    // 1. Create a mutation
    final created = await mutationManager.create(
      RecordId(table: "user", key: "1"),
      {"name": "Alice"},
    );

    // 2. Queue should populate via listener
    // EventSystem is async (microtask), so wait for it
    await Future.delayed(Duration.zero);
    expect(queue.size, 1);

    // 3. Process the queue
    bool processed = false;
    await queue.next((event) async {
      processed = true;
      expect(event is CreateEvent, true);
      expect(event.recordId, contains("user:`1`"));
    });

    expect(processed, true);
    expect(queue.size, 0); // Should be removed from queue

    // 4. Verify removal from DB
    final pending = await localService.getClient.query(
      sql: "SELECT * FROM _spooky_pending_mutations",
    ); // Should be empty
    // Note: extractResult helper might be needed if result is complex, but raw check works if empty
    // Actually depending on implementation, extractResult logic in queue_up might imply specific structure.
    // If we assume empty list or null depending on wrapper.
    expect(pending.toString(), contains("[]")); // Rough check for empty list

    // 5. Test Load from DB
    // Manually insert a mutation record
    await localService.getClient.query(
      sql:
          "CREATE _spooky_pending_mutations SET mutationType = 'update', recordId = 'user:2', data = {name: 'Bob'}",
    );

    await queue.loadFromDatabase();
    expect(queue.size, 1);

    // Process it
    await queue.next((event) async {
      expect(event is UpdateEvent, true);
    });
    expect(queue.size, 0);

    await localService.close();
  });
}
