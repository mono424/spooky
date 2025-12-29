import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    await RustLib.init();
  });

  testWidgets('Live Query Snapshot Verification Test (Standalone)', (
    WidgetTester tester,
  ) async {
    print('\n🚀 STARTING LIVE QUERY VERIFICATION TEST (STANDALONE) 🚀\n');

    // 1. Initialize DB Directly
    final db = await SurrealDb.connect(
      mode: StorageMode.devSidecar(path: 'test_db_verification', port: 5700),
    );
    await db.useDb(ns: "test_verification_ns", db: "test_verification_db");

    // 2. Setup: Ensure User table is empty or known state
    print('TEST: Cleaning up table...');
    try {
      await db.query(sql: "REMOVE TABLE user");
    } catch (_) {}
    await db.query(sql: "DEFINE TABLE user SCHEMALESS");

    // 3. Start Live Query (Snapshot Mode)
    print('TEST: Subscribing to select("user").live()...');
    final query = db.select(resource: 'user');
    final stream = query.live<Map<String, dynamic>>((json) => json);

    final Completer<List<Map<String, dynamic>>> firstSnapshot = Completer();
    final Completer<List<Map<String, dynamic>>> secondUpdate = Completer();
    final Completer<List<Map<String, dynamic>>> thirdDelete = Completer();

    int eventCount = 0;

    final subscription = stream.listen(
      (users) {
        print(
          'TEST: 🟢 Received Stream Update #${eventCount + 1}: ${users.length} items',
        );

        if (!firstSnapshot.isCompleted) {
          firstSnapshot.complete(users);
        } else if (!secondUpdate.isCompleted) {
          secondUpdate.complete(users);
        } else if (!thirdDelete.isCompleted) {
          thirdDelete.complete(users);
        }
        eventCount++;
      },
      onError: (e) {
        print('TEST: 🛑 Stream Error: $e');
      },
    );

    // 4. Verify Initial Snapshot (Should be empty)
    print('TEST: Waiting for Initial Snapshot...');
    final initialList = await firstSnapshot.future.timeout(
      const Duration(seconds: 5),
    );
    print('TEST: Initial Snapshot Verified. Size: ${initialList.length}');
    expect(
      initialList.isEmpty,
      isTrue,
      reason: "Snapshot should be empty initially",
    );

    // Give time for stream registration
    await Future.delayed(const Duration(seconds: 1));

    // 5. Create Item
    final testId = "test_user_${Random().nextInt(9999)}";
    final data = {
      // NOTE: We provide just the ID part. SurrealDB combined with table 'user' creates 'user:test_user_...'
      "id": testId,
      "username": "User $testId",
      "status": "alive",
    };
    print('TEST: Creating user $testId...');
    await db.create(resource: 'user', data: jsonEncode(data));

    // 6. Verify Update
    print('TEST: Waiting for Create Event in Stream...');
    final updatedList = await secondUpdate.future.timeout(
      const Duration(seconds: 5),
    );
    print('TEST: Update Verified. Size: ${updatedList.length}');
    expect(updatedList.length, equals(1));
    expect(updatedList.first['id'], equals("user:$testId"));

    // 7. Delete Item
    print('TEST: Deleting user $testId...');
    await db.delete(resource: "user:$testId");

    // 8. Verify Delete
    print('TEST: Waiting for Delete Event in Stream...');
    final finalList = await thirdDelete.future.timeout(
      const Duration(seconds: 5),
    );
    print('TEST: Delete Verified. Size: ${finalList.length}');
    expect(finalList.isEmpty, isTrue);

    // Cleanup
    await subscription.cancel();
    await db.close();
    print('\n✅ TEST COMPLETED SUCCESSFULLY ✅\n');
  });
}
