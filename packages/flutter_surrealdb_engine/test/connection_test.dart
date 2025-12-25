import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/api/client.dart';
import 'dart:io';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

void main() {
  setUpAll(() async {
    // Explicitly load the dylib for testing since we are not in a full app bundle
    String? dylibPath;
    if (Platform.isMacOS) {
      dylibPath =
          '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
    } else if (Platform.isLinux) {
      dylibPath = 'rust/target/debug/librust_lib_surrealdb.so';
    } else if (Platform.isWindows) {
      dylibPath = 'rust/target/debug/rust_lib_surrealdb.dll';
    }

    if (dylibPath != null) {
      final lib = ExternalLibrary.open(dylibPath);
      await RustLib.init(externalLibrary: lib);
    } else {
      await RustLib.init();
    }
  });

  test('Connect to local SurrealDB v3 via FFI', () async {
    print('Attempting to connect to ws://127.0.0.1:8000...');

    // Create the client directly via FFI wrapper if possible, or use the high level class
    try {
      final client = await SurrealDb.connect(
        mode: StorageMode.remote(url: 'ws://127.0.0.1:8000'),
      );
      print('Connected successfully!');

      print('Attempting query...');
      final result = await client.query(sql: 'INFO FOR DB;', vars: '{}');
      print('Query Result: $result');

      expect(result, isNotNull);
      // Expecting valid JSON array as string
      expect(result, contains('['));
    } catch (e) {
      print('Connection failed: $e');
      fail('Connection failed: $e');
    }
  });
}
