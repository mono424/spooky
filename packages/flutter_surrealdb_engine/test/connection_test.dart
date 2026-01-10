import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/api/client.dart';
import 'dart:convert';
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
        mode: StorageMode.remote(url: 'ws://127.0.0.1:8000/rpc'),
      );
      print('Connected successfully!');

      try {
        print('Attempting SIGNUP with valid data...');
        final token = await client.signup(
          credentialsJson: jsonEncode({
            "ns": "main",
            "db": "main",
            "access": "account",
            "username": "test_user_123",
            "password": "secure_password",
          }),
        );
        print('Signup success! Token: ${token.substring(0, 20)}...');
      } catch (e) {
        print('Signup failed: $e');
      }

      try {
        print('Attempting SIGNUP with SHORT username...');
        await client.signup(
          credentialsJson: jsonEncode({
            "ns": "main",
            "db": "main",
            "access": "account", // Scope
            "username": "ABC", // Too short
            "password": "pass",
          }),
        );
      } catch (e) {
        print('Short username signup failed as expected: $e');
      }

      String? result;
      try {
        print('Attempting ROOT signin...');
        // Using Root credentials
        await client.signin(
          credentialsJson: jsonEncode({"user": "root", "pass": "root"}),
        );
        print('Root signin successful.');

        print('Attempting to use ns/db...');
        await client.useDb(namespace: "main", database: "main");

        print('Attempting manual CREATE user...');
        // ...

        print('Checking if ALL users...');
        final check2 = await client.query(
          sql: "SELECT * FROM user;",
          vars: "{}",
        );
        print('All Users: $check2');

        print('Overwriting ACCESS account with debug SIGNUP...');
        await client.query(
          sql:
              '''
              REMOVE ACCESS account ON DATABASE;
              DEFINE ACCESS account ON DATABASE TYPE RECORD
              SIGNUP {
                LET $test = (SELECT * FROM user LIMIT 1);
                RETURN true;
              }
            ''',
          vars: "{}",
        );
        print('ACCESS account overwritten.');

        print('Attempting SIGNUP with valid data (AGAIN)...');
        final token = await client.signup(
          credentialsJson: jsonEncode({
            "ns": "main",
            "db": "main",
            "access": "account",
            "username": "test_user_FIXED_FINAL",
            "password": "secure_password",
          }),
        );
        print('Signup success with FIX! Token: ${token.substring(0, 20)}...');
      } catch (e) {
        print('Manual Create failed: $e');
      }

      expect(result, isNotNull);
      // Expecting valid JSON array as string
      expect(result, contains('['));
    } catch (e) {
      print('Connection failed: $e');
      fail('Connection failed: $e');
    }
  });
}
