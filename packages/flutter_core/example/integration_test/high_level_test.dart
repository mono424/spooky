import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('High Level API Test', (WidgetTester tester) async {
    await RustLib.init();
    final db = await SurrealDb.connect(mode: StorageMode.memory());
    await db.useDb(ns: "test_ns", db: "test_db");

    final tableName = "users";
    await db.query(sql: "DEFINE TABLE $tableName SCHEMALESS");

    // 1. Create Initial Data
    await db.create(resource: tableName, data: jsonEncode({"name": "Alice"}));
    await db.create(resource: tableName, data: jsonEncode({"name": "Bob"}));

    // 2. Verify Backward Compatibility (await select)
    final snapshotJson = await db.select(resource: tableName);
    print("Snapshot JSON: $snapshotJson");
    expect(snapshotJson, contains("Alice"));
    expect(snapshotJson, contains("Bob"));

    // 3. Verify Live Query
    final completer = Completer<List<Map<String, dynamic>>>();

    // Model for testing
    Map<String, dynamic> fromJson(Map<String, dynamic> json) => json;

    final stream = db.select(resource: tableName).live(fromJson);

    List<Map<String, dynamic>> lastList = [];
    int buildCount = 0;

    final subscription = stream.listen((list) {
      print("Stream update: $list");
      lastList = list;
      buildCount++;

      // Check if we have both users (Snapshot)
      if (buildCount == 1) {
        if (list.length == 2) {
          print("Verified Snapshot received.");
        }
      }

      // Check for new user
      if (list.any((u) => u['name'] == 'Charlie')) {
        if (!completer.isCompleted) completer.complete(list);
      }
    });

    // Allow snapshot to arrive
    await Future.delayed(Duration(seconds: 1));
    expect(lastList.length, 2);

    // 4. Trigger Create
    print("Creating Charlie...");
    await db.create(resource: tableName, data: jsonEncode({"name": "Charlie"}));

    // 5. Verify Update
    try {
      final finalList = await completer.future.timeout(Duration(seconds: 5));
      expect(finalList.length, 3);
      expect(finalList.any((u) => u['name'] == 'Charlie'), isTrue);
    } catch (e) {
      fail("Did not receive Charlie update: $e");
    }

    // 6. Trigger Delete
    print("Deleting Alice...");
    // Find Alice's ID
    final alice = lastList.firstWhere((u) => u['name'] == 'Alice');
    await db.delete(resource: alice['id']);

    await Future.delayed(Duration(seconds: 1));
    expect(lastList.length, 2);
    expect(lastList.any((u) => u['name'] == 'Alice'), isFalse);

    await subscription.cancel();
    db.close();
  });
}
