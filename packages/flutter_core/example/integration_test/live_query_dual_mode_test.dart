import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    await RustLib.init();
  });

  Future<void> cleanTable(SurrealDb db, String table) async {
    try {
      await db.query(sql: "REMOVE TABLE $table");
    } catch (_) {}
    await db.query(sql: "DEFINE TABLE $table SCHEMALESS");
  }

  testWidgets('Dual Mode 1: Pure Stream (Legacy/Reference)', (
    WidgetTester tester,
  ) async {
    final db = await SurrealDb.connect(
      mode: StorageMode.devSidecar(path: 'test_db_dual_1', port: 5700),
    );
    await db.useDb(ns: "test_ns", db: "test_db");
    final tableName = "user_pure_stream";
    await cleanTable(db, tableName);

    final createCompleter = Completer<LiveQueryEvent>();

    print(
      "TEST [Pure]: Subscribing to $tableName (Snapshot should be FALSE default)...",
    );
    final stream = db.liveQuery(
      tableName: tableName,
    ); // Defaults to snapshot: false

    final subscription = stream.listen((event) {
      if (event.action == LiveQueryAction.snapshot) {
        fail("TEST FAILED: Received unexpected Snapshot in Pure Mode!");
      }
      if (event.action == LiveQueryAction.create) {
        if (!createCompleter.isCompleted) createCompleter.complete(event);
      }
    });

    // Create event
    print("TEST [Pure]: Creating record...");
    // await db.query(sql: "CREATE $tableName SET name = 'Pure User'");
    await db.create(
      resource: tableName,
      data: jsonEncode({"name": "Pure User"}),
    );

    try {
      await createCompleter.future.timeout(const Duration(seconds: 5));
      print("TEST [Pure]: Event Received ✅");
    } catch (e) {
      fail("TEST FAILED: Pure stream timeout. Error: $e");
    }

    await subscription.cancel();
    await db.close();
  });

  testWidgets('Dual Mode 2: Snapshot + Stream (New API)', (
    WidgetTester tester,
  ) async {
    final db = await SurrealDb.connect(
      mode: StorageMode.devSidecar(path: 'test_db_dual_2', port: 5700),
    );
    await db.useDb(ns: "test_ns", db: "test_db");
    final tableName = "user_snapshot_stream";
    await cleanTable(db, tableName);

    // Seeding for Snapshot
    print("TEST [Snapshot]: Seeding record...");
    await db.create(
      resource: tableName,
      data: jsonEncode({"name": "Seeded User"}),
    );

    final snapshotCompleter = Completer<List<dynamic>>();
    final createCompleter = Completer<dynamic>();

    print("TEST [Snapshot]: Subscribing via select().live()...");

    // Using select().live() helper which returns Stream<List<T>>
    // This internally uses snapshot=true
    final stream = db
        .select(resource: tableName)
        .live<Map<String, dynamic>>((json) => json);

    final subscription = stream.listen((list) {
      print("TEST [Snapshot]: List Updated: ${list.length} items");

      // Check for seeded item
      if (list.any((item) => item.values.contains('Seeded User')) &&
          !snapshotCompleter.isCompleted) {
        snapshotCompleter.complete(list);
      }

      // Check for new item
      if (list.any((item) => item.values.contains('Live User')) &&
          !createCompleter.isCompleted) {
        createCompleter.complete(list);
      }
    });

    // Verify Snapshot
    try {
      await snapshotCompleter.future.timeout(const Duration(seconds: 5));
      print("TEST [Snapshot]: Snapshot Verified ✅");
    } catch (e) {
      fail("TEST FAILED: Snapshot timeout. Error: $e");
    }

    // Give the sidecar a moment to register the live query stream fully
    // This prevents the race condition where we create the record before the subscription is active on the server
    await Future.delayed(const Duration(milliseconds: 1000));

    // Trigger Live Event
    print("TEST [Snapshot]: Creating Live User...");
    // await db.query(sql: "CREATE $tableName SET name = 'Live User'");
    await db.create(
      resource: tableName,
      data: jsonEncode({"name": "Live User"}),
    );

    // Verify Create
    try {
      await createCompleter.future.timeout(const Duration(seconds: 5));
      print("TEST [Snapshot]: Live Update Verified ✅");
    } catch (e) {
      fail("TEST FAILED: Live update timeout. Error: $e");
    }

    await subscription.cancel();
    await db.close();
  });
}
