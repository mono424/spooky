import 'dart:io';
import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_core/core/spooky.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_core/core/services/database/remote.dart';
import 'package:flutter_core/core/types.dart'; // Contains QueryTimeToLive

// Relative import to example schema
import '../example/lib/schema/src/models.dart';

void main() {
  test('SpookySync Full Sync Verification Test', () async {
    print('Starting Full Sync Verification Test...');

    // 1. Initialize Rust
    final dylibPath =
        '../flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    if (File(dylibPath).existsSync()) {
      try {
        await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));
      } catch (_) {}
    } else {
      try {
        await RustLib.init();
      } catch (_) {}
    }

    // 2. Seed Remote Data
    print('Seeding remote data...');
    final seedClient = await SurrealDb.connect(
      mode: StorageMode.remote(url: 'ws://localhost:8666/rpc'),
    );

    // Auth - Using Root for reliable seeding
    await seedClient.signin(
      creds: jsonEncode({"user": "root", "pass": "root"}),
    );
    await seedClient.useDb(ns: 'main', db: 'main');

    // Cleanup & Seed
    await seedClient.query(sql: 'DELETE user; DELETE thread;');

    // Seed User
    final userId = 'user:sync_test_user';
    await seedClient.query(
      sql:
          'CREATE $userId SET username = "sync_master", password = "hashed_password"',
    );

    // Seed Threads
    final thread1Id = 'thread:sync_test_1';
    final thread2Id = 'thread:sync_test_2';

    await seedClient.query(
      sql:
          'CREATE $thread1Id SET title = "Sync Thread 1", content = "Content 1", author = $userId, created_at = time::now()',
    );
    await seedClient.query(
      sql:
          'CREATE $thread2Id SET title = "Sync Thread 2", content = "Content 2", author = $userId, created_at = time::now()',
    );

    print('Seeding complete.');

    // 3. Initialize SpookyClient
    final schemaJson = jsonEncode({
      'relationships': [
        {'from': 'thread', 'field': 'author', 'to': 'user'},
      ],
    });

    final config = SpookyConfig(
      schemaSurql: SURQL_SCHEMA, // Pass schema for provisioning internal tables
      schema: schemaJson,
      database: DatabaseConfig(
        endpoint: 'ws://localhost:8666/rpc',
        path: ':memory:',
        namespace: 'main',
        database: 'main',
        token: null, // Avoid bad placeholder auth
      ),
      enableLiveQuery: false,
    );

    print('Initializing SpookyClient...');
    final client = await SpookyClient.init(config);
    print('SpookyClient initialized.');

    // 4. Authenticate SpookyClient
    await client.useRemote((c) async {
      await c.signin(creds: jsonEncode({"user": "root", "pass": "root"}));
    });
    print('Authenticated as test user.');

    // Explicitly define thread table locally to avoid "table does not exist" error during testing
    try {
      await client.localClient.query(sql: 'DEFINE TABLE thread SCHEMALESS;');
      print('Defined thread table locally.');
    } catch (e) {
      print('Warning: Failed to define thread table: $e');
    }

    // 5. Register Query
    print('Registering Query for Threads...');
    await client.query(
      tableName: 'thread',
      surrealql: 'SELECT * FROM thread ORDER BY title ASC',
      params: {},
    );

    // 6. Verification
    print('Polling local DB for synced threads...');

    bool found = false;
    for (int i = 0; i < 40; i++) {
      await Future.delayed(Duration(milliseconds: 500));

      try {
        final resStr = await client.localClient.query(
          sql: 'SELECT * FROM thread',
        );
        // Engine returns JSON string
        final dynamic raw = jsonDecode(resStr);

        // Handle various result shapes robustly
        List<dynamic> rows = [];

        if (raw is List && raw.isNotEmpty) {
          final first = raw[0];
          if (first is Map && first['status'] == 'OK') {
            if (first['result'] is List) {
              rows = first['result'];
            }
          } else if (first is Map && first.containsKey('result')) {
            // Fallback
            if (first['result'] is List) rows = first['result'];
          }
        } else if (raw is Map && raw['status'] == 'OK') {
          if (raw['result'] is List) rows = raw['result'];
        }

        if (rows.isNotEmpty) {
          print('Poll $i: Found ${rows.length} threads');
          // Debug print first row
          // print('Row 0: ${rows[0]}');

          if (rows.length >= 2) {
            final t1 = rows.firstWhere(
              (r) => r is Map && r['id'] == thread1Id,
              orElse: () => null,
            );
            final t2 = rows.firstWhere(
              (r) => r is Map && r['id'] == thread2Id,
              orElse: () => null,
            );

            if (t1 != null && t2 != null) {
              print('Success! Threads synced.');
              found = true;
              break;
            }
          }
        }
      } catch (e) {
        print('Poll error: $e');
      }
    }

    await client.close();

    if (!found) {
      fail('Failed to sync threads from remote within timeout.');
    }
  });
}
