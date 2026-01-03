import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_core/core/services/mutation/main.dart';
import 'package:flutter_core/core/services/mutation/events.dart';
import 'package:flutter_core/core/types.dart' hide RecordId;

void main() {
  test('MutationManager integration test with Local DB and CamelCase Schema', () async {
    final dylibPath =
        '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    try {
      await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));
    } catch (_) {
      // Init might fail if already initialized in same process
    }

    final config = DatabaseConfig(
      path: ":memory:",
      namespace: "test_ns",
      database: "test_db",
    );

    final localService = await LocalDatabaseService.create(config);
    await localService.init();

    // Use SCHEMALESS to match 'models.dart' implies flexibility or alignment.
    // And use camelCase names as in models.dart
    // Clear potentially lingering schema
    try {
      await localService.getClient.query(
        sql: "REMOVE TABLE _spooky_pending_mutations;",
      );
    } catch (_) {}
    try {
      await localService.getClient.query(sql: "REMOVE TABLE user;");
    } catch (_) {}

    await localService.getClient.query(
      sql:
          "DEFINE TABLE _spooky_pending_mutations SCHEMALESS; DEFINE FIELD mutationType ON TABLE _spooky_pending_mutations TYPE string; DEFINE FIELD recordId ON TABLE _spooky_pending_mutations TYPE any; DEFINE FIELD data ON _spooky_pending_mutations TYPE any;",
    );
    await localService.getClient.query(
      sql:
          "DEFINE TABLE user SCHEMALESS; DEFINE FIELD name ON TABLE user TYPE string;",
    );

    final mutationManager = MutationManager(localService);

    // 1. Test Create
    print("Testing Create...");
    // Use queryTyped if available/needed or just create
    final created = await mutationManager.create(
      RecordId(table: "user", key: "1"),
      {"name": "Alice"},
    );
    expect(created.target!.record['name'], "Alice");

    // Check pending mutation using camelCase fields
    final pendingRaw = await localService.getClient.query(
      sql: "SELECT * FROM _spooky_pending_mutations",
    );
    // Should contain mutationType='create'
    expect(pendingRaw, contains("create"));
    // Key "1" as string results in user:`1`
    expect(pendingRaw.toString().contains("user:`1`"), isTrue);

    // 2. Test Update
    print("Testing Update...");
    final updated = await mutationManager.update(
      RecordId(table: "user", key: "1"),
      {"name": "Bob"},
    );
    expect(updated.target!.record['name'], "Bob");

    // 3. Test Delete
    print("Testing Delete...");
    await mutationManager.delete(RecordId(table: "user", key: "1"));
    // Ensure we clear previous list or check specifically for delete
    final pendingDelete = await localService.getClient.query(
      sql:
          "SELECT * FROM _spooky_pending_mutations WHERE mutationType = 'delete'",
    );
    expect(pendingDelete, contains("delete"));
    // Check for user:`1` in delete recordId as well
    expect(pendingDelete.toString().contains("user:`1`"), isTrue);

    await localService.close();
  });
}
