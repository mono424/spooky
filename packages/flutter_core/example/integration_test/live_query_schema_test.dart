import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('Live Query SCHEMAFULL Reproduction', (
    WidgetTester tester,
  ) async {
    await RustLib.init();
    final db = await SurrealDb.connect(
      mode: StorageMode.devSidecar(path: 'test_db_schema', port: 5600),
    );

    // 1. Setup Database & Schema (Mirroring App)
    await db.useDb(ns: "test_ns", db: "test_db");

    // Define SCHEMAFULL table with restrictive permissions (like App)
    // NOTE: We are ROOT, so we should bypass this.
    await db.query(
      sql: """
      DEFINE TABLE user_schema SCHEMAFULL
        PERMISSIONS 
          FOR select, update, delete, create WHERE false -- strict blocking
      ;
      DEFINE FIELD name ON TABLE user_schema TYPE string;
    """,
    );

    final completer = Completer<LiveQueryEvent>();
    final tableName = "user_schema";

    print("TEST: Subscribing to $tableName (SCHEMAFULL)");
    final stream = db.liveQuery(tableName: tableName);

    final subscription = stream.listen((event) {
      print("TEST: Event: ${event.action} ${event.result}");
      if (event.action == LiveQueryAction.create && !completer.isCompleted) {
        completer.complete(event);
      }
    }, onError: (e) => print("TEST: Error: $e"));

    // 2. Trigger Event (As Root)
    print("TEST: Creating record in $tableName");
    // Ensure we send valid data for schema
    await db.query(sql: "CREATE $tableName SET name = 'Test User'");

    // 3. Verify
    try {
      final event = await completer.future.timeout(const Duration(seconds: 5));
      print("TEST: SUCCESS - Received event despite SCHEMAFULL");
      expect(event.result, contains("Test User"));
    } catch (e) {
      fail(
        "TEST FAILED: Timeout waiting for event on SCHEMAFULL table. Bug confirmed.",
      );
    } finally {
      await subscription.cancel();
      db.close();
    }
  });
}
