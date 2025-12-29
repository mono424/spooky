import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';
import 'package:path_provider/path_provider.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('Live Query receives CREATE events', (WidgetTester tester) async {
    await RustLib.init();
    // 1. Setup Database
    // Connect to embedded database
    // Connect to devSidecar
    final db = await SurrealDb.connect(mode: StorageMode.memory());

    // Auth: For embedded KV, we might have implicit root access or no auth required initially.
    // Try skipping auth.

    // Use a specific namespace/database
    await db.useDb(ns: "test_ns", db: "test_db");

    // 2. Setup Live Query
    final completer = Completer<LiveQueryEvent>();
    final tableName = "test_user_${DateTime.now().millisecondsSinceEpoch}";

    // Define table first to avoid "table does not exist" error
    await db.query(sql: "DEFINE TABLE $tableName SCHEMALESS");

    print("TEST: Subscribing to $tableName");
    final stream = db.liveQuery(tableName: tableName);

    final subscription = stream.listen(
      (event) {
        print(
          "TEST: Received Event: ${event.action} - ${event.result} (UUID: ${event.queryUuid})",
        );

        if (event.action == LiveQueryAction.unknown &&
            event.queryUuid != null) {
          print("TEST: Handshake received with UUID: ${event.queryUuid}");
          return;
        }

        if (event.action == LiveQueryAction.create && !completer.isCompleted) {
          completer.complete(event);
        }
      },
      onError: (e) {
        print("TEST: Stream Error: $e");
        if (!completer.isCompleted) completer.completeError(e);
      },
      onDone: () {
        print("TEST: Stream Closed (onDone)");
        if (!completer.isCompleted)
          completer.completeError("Stream closed before receiving event");
      },
    );

    // 3. Trigger Event
    print("TEST: Creating record in $tableName");
    final createQuery = "CREATE $tableName SET name = 'Test User'";
    await db.query(sql: createQuery);

    // 4. Verify Event Received
    try {
      final event = await completer.future.timeout(const Duration(seconds: 5));
      print("TEST: Verified event received: ${event.action} ${event.result}");
      expect(event.action, LiveQueryAction.create);
      expect(event.result, contains("Test User"));
    } catch (e) {
      fail(
        "TEST FAILED: Did not receive live query event within 5 seconds. Error: $e",
      );
    } finally {
      print("TEST: Cancelling subscription...");
      try {
        await subscription.cancel().timeout(
          const Duration(seconds: 2),
          onTimeout: () {
            print(
              "TEST: Subscription cancel timed out (expected if stream is blocked)",
            );
          },
        );
      } catch (e) {
        print("TEST: Error during cancel: $e");
      }
      // Wait for cancellation to propagate (testing the fix)
      await Future.delayed(const Duration(seconds: 1));
      db.close();
    }
  });
}
