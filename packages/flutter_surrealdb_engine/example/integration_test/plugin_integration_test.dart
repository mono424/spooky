// This is a basic Flutter integration test.
//
// Since integration tests run in a full Flutter application, they can interact
// with the host side of a plugin implementation, unlike Dart unit tests.
//
// For more information about Flutter integration tests, please see
// https://flutter.dev/to/integration-testing

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:path_provider/path_provider.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  group('SurrealDB Engine Tests', () {
    SurrealDatabase? db;
    late String dbPath;

    setUpAll(() async {
      print('Initializing RustLib...');
      await RustLib.init();
      print('RustLib initialized.');

      final tempDir = await getTemporaryDirectory();
      dbPath =
          '${tempDir.path}/test_surreal_${DateTime.now().millisecondsSinceEpoch}.db';
      print('DB Path: $dbPath');
      // Clean up previous test run if needed (unlikely with timestamp)
      final file = File(dbPath);
      if (await file.exists()) {
        await file.delete();
      }

      print('Connecting to DB...');

      try {
        db = await connectDb(path: dbPath);
        print('Connected to DB.');
      } catch (e) {
        fail('Error connecting to DB: $e');
      }
    });

    testWidgets('Check Connection', (WidgetTester tester) async {
      expect(db, isNotNull);
    });

    testWidgets('Health Check', (WidgetTester tester) async {
      expect(db, isNotNull);
      await db!.health();
    });

    testWidgets('Version Check', (WidgetTester tester) async {
      expect(db, isNotNull);
      final versionResults = await db!.version();
      expect(versionResults.length, 1);
      final version = versionResults.first.result;
      expect(version, isNotEmpty);
      print('SurrealDB Version: $version');
    });

    testWidgets('Authentication (Root)', (WidgetTester tester) async {
      expect(db, isNotNull);
      // Define user first
      await db!.queryDb(
        query: "DEFINE USER root ON ROOT PASSWORD 'root' ROLES OWNER;",
      );

      final results = await db!.signinRoot(username: 'root', password: 'root');
      expect(results.length, 1);
      final token = results.first.result;
      expect(token, isNotNull);
      expect(token, isNotEmpty);
    });

    testWidgets('Session Management', (WidgetTester tester) async {
      expect(db, isNotNull);
      await db!.useNs(ns: 'test_ns');
      await db!.useDb(db: 'test_db');

      final result = await db!.queryDb(query: 'RETURN session::ns();');
      expect(result.first.result, contains('test_ns'));
    });

    testWidgets('Authentication Flow', (WidgetTester tester) async {
      expect(db, isNotNull);
      // Define user first
      await db!.queryDb(
        query: "DEFINE USER root ON ROOT PASSWORD 'root' ROLES OWNER;",
      );

      // Signin
      final signinResults = await db!.signinRoot(
        username: 'root',
        password: 'root',
      );
      expect(signinResults.length, 1);
      final tokenJson = signinResults.first.result;
      expect(tokenJson, isNotNull);
      print('Signin Token JSON: $tokenJson');

      // Authenticate
      // The tokenJson is likely a JSON string (quoted) or object.
      // We should parse it.
      try {
        final decoded = jsonDecode(tokenJson!);
        final String token = decoded is String ? decoded : tokenJson;
        print('Decoded Token: $token');

        final authResults = await db!.authenticate(token: token);
        expect(authResults.length, 1);
        expect(authResults.first.status, 'OK');
      } catch (e) {
        print('Authentication failed with parsed token: $e');
        fail('Authentication failed: $e');
      }
    });

    testWidgets('CRUD Operations', (WidgetTester tester) async {
      expect(db, isNotNull);
      // Use Namespace and Database
      await db!.useNs(ns: 'test_ns');
      await db!.useDb(db: 'test_db');

      // Create
      final createData = "{ \"name\": \"Test User\", \"age\": 30 }";
      final createQuery = "CREATE person CONTENT $createData";
      final createResults = await db!.queryDb(query: createQuery);
      expect(createResults.length, 1);
      final createResult = createResults.first;
      expect(createResult.status, 'OK');
      expect(createResult.result, contains('Test User'));

      // Extract ID
      // Result is verbose JSON from SurrealDB Rust serialization
      // e.g. {"Array":[{"Object":{"id":{"Thing":{"tb":"person","id":{"String":"..."}}}, ...}}]}
      final resultStr = createResult.result!;
      final json = jsonDecode(resultStr);
      String? id;
      try {
        // Traverse the verbose structure
        // We expect an Array of results
        if (json is Map && json.containsKey('Array')) {
          final array = json['Array'] as List;
          if (array.isNotEmpty) {
            final firstObj = array[0];
            if (firstObj is Map && firstObj.containsKey('Object')) {
              final obj = firstObj['Object'];
              if (obj is Map && obj.containsKey('id')) {
                final idObj = obj['id'];
                if (idObj is Map && idObj.containsKey('Thing')) {
                  final thing = idObj['Thing'];
                  // Construct the full ID "tb:id"
                  final tb = thing['tb'];
                  final idPartObj = thing['id'];
                  String? idPart;
                  if (idPartObj is Map && idPartObj.containsKey('String')) {
                    idPart = idPartObj['String'];
                  }
                  if (tb != null && idPart != null) {
                    id = '$tb:$idPart';
                  }
                }
              }
            }
          }
        }
      } catch (e) {
        print('Error parsing JSON: $e');
      }

      if (id == null) {
        fail('Could not extract ID from: $resultStr');
      }
      expect(id, isNotNull);

      // Select
      final selectResults = await db!.queryDb(query: "SELECT * FROM person");
      expect(selectResults.length, 1);
      expect(selectResults.first.result, contains('Test User'));

      // Update
      final updateData = "{ \"name\": \"Updated User\", \"age\": 31 }";
      final updateQuery = "UPDATE $id CONTENT $updateData";
      final updateResults = await db!.queryDb(query: updateQuery);
      expect(updateResults.length, 1);
      expect(updateResults.first.status, 'OK');
      expect(updateResults.first.result, contains('Updated User'));

      // Merge
      final mergeData = "{ \"active\": true }";
      final mergeQuery = "UPDATE $id MERGE $mergeData";
      final mergeResults = await db!.queryDb(query: mergeQuery);
      expect(mergeResults.length, 1);
      expect(mergeResults.first.status, 'OK');
      expect(mergeResults.first.result, contains('active'));
      expect(mergeResults.first.result, contains('Updated User'));

      // Delete
      final deleteQuery = "DELETE $id";
      final deleteResults = await db!.queryDb(query: deleteQuery);
      expect(deleteResults.length, 1);
      expect(deleteResults.first.status, 'OK');

      // Verify Delete
      final verifyQuery = "SELECT * FROM person";
      final verifyTableResults = await db!.queryDb(query: verifyQuery);
      expect(verifyTableResults.length, 1);
      // The result should be an empty array or null-ish representation in verbose JSON
      // e.g. {"Array":[]}
      final verifyResultStr = verifyTableResults.first.result!;
      if (verifyResultStr.contains(id!)) {
        // Double check if it's really the ID or just a string match
        // But if it contains the ID, it might still exist
        // Let's parse it
        final verifyJson = jsonDecode(verifyResultStr);
        if (verifyJson is Map && verifyJson.containsKey('Array')) {
          final array = verifyJson['Array'] as List;
          if (array.isNotEmpty) {
            fail('Record $id still exists after delete');
          }
        }
      }
    });
  });
}
