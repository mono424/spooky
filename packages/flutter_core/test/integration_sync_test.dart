import 'dart:io';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/core/spooky.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

// No longer importing models.dart for schema string

void main() {
  test('Integration: Spooky Sync (Local -> Remote)', () async {
    // 0. Manual Library Load
    final dylibPath = '../flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    if (!File(dylibPath).existsSync()) {
      fail('Rust dylib not found at $dylibPath');
    }
    
    try {
      await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));
    } catch (_) { }

    // Read schemas
    final remoteSchemaFile = File('../../example/schema/src/schema.surql'); 
    
    if (!remoteSchemaFile.existsSync()) {
       if (File('example/schema/src/schema.surql').existsSync()) {
       }
    }
    
    String remoteSchemaContent = "";
    try {
        remoteSchemaContent = File('../../example/schema/src/schema.surql').readAsStringSync();
    } catch (_) {
        try {
            remoteSchemaContent = File('example/lib/schema/schema.surql').readAsStringSync(); // Fallback to local copy if main not found
            print('Warning: Using local schema copy instead of repo root schema.');
        } catch (e) {
            fail('Could not load remote schema: $e');
        }
    }

    final localLooseSchema = '''
      DEFINE TABLE user SCHEMALESS;
      DEFINE TABLE thread SCHEMALESS;
      DEFINE TABLE comment SCHEMALESS;
      DEFINE TABLE _spooky_pending_mutations SCHEMALESS;
    ''';


    // ==============================================================================
    // PHASE 1: SKIPPED (No Root Access)
    // ==============================================================================
    print('\n=== PHASE 1: SKIPPED (No Root Access) ===');
    // Using existing environment 'main' / 'main' with pre-provisioned schema.


    // ==============================================================================
    // PHASE 2: SYNC VALIDATION
    // ==============================================================================
    print('\n=== PHASE 2: SYNC VALIDATION ===');

    final config = SpookyConfig(
      schemaSurql: localLooseSchema,
      schema: '', 
      database: DatabaseConfig(
        path: '${Directory.current.path}/test_db_phase2_${DateTime.now().millisecondsSinceEpoch}',
        namespace: 'main',
        database: 'main',
        endpoint: 'ws://localhost:8666', 
        token: null, 
      ),
    );

    final client = await SpookyClient.init(config);
    
    // Auth
    final token = await client.remote.client!.signin(creds: '{"ns":"main","db":"main","access":"account","username":"test","password":"123"}');
    await client.authenticate(token);

    final threadId = 'thread:sync_${DateTime.now().millisecondsSinceEpoch}';
    final commentId = 'comment:sync_${DateTime.now().millisecondsSinceEpoch}';
    // Use the ID provided by the user to verify their specific case
    final userId = 'user:rvkme6hk9ckgji6dlcvx';

    print('Creating thread: $threadId');
    await client.create(RecordId.fromString(threadId), {
      'title': 'Sync Thread',
      'content': 'Content',
      'author': RecordId.fromString(userId), // Using RecordId Object!
    });
    
    print('Creating comment: $commentId');
    await client.create(RecordId.fromString(commentId), {
      'content': 'Sync Comment',
      'thread': RecordId.fromString(threadId), // Using RecordId Object!
      'author': RecordId.fromString(userId),   // Using RecordId Object!
    });

    print('Waiting for sync...');
    await Future.delayed(const Duration(seconds: 4));

    // Verify
    print('Verifying remote...');
    final remoteThread = await client.remote.client?.select(resource: threadId);
    print('Remote Thread: $remoteThread');

    final remoteComment = await client.remote.client?.select(resource: commentId);
    print('Remote Comment: $remoteComment');

    if (remoteThread == null || remoteThread.contains('[]')) {
        fail('Sync Failed: Remote thread not found.');
    }

    await client.close();
    final dbDir = Directory(config.database.path);
    if (await dbDir.exists()) await dbDir.delete(recursive: true);
  });
}
