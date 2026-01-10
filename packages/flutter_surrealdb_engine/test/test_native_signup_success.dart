import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/api/client.dart';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_surrealdb_engine/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

void main() {
  setUpAll(() async {
    String? dylibPath;
    if (Platform.isMacOS) {
      dylibPath =
          '${Directory.current.path}/rust/target/debug/librust_lib_surrealdb.dylib';
    }
    if (dylibPath != null) {
      final lib = ExternalLibrary.open(dylibPath);
      await RustLib.init(externalLibrary: lib);
    } else {
      await RustLib.init();
    }
  });

  test('Native Signup with Fixed Schema', () async {
    print('Connecting to live server at ws://127.0.0.1:8000/rpc...');

    final client = await SurrealDb.connect(
      mode: StorageMode.remote(url: 'ws://127.0.0.1:8000/rpc'),
    );
    print('Connected.');

    try {
      final username =
          "native_user_fixed_${DateTime.now().millisecondsSinceEpoch}";
      final creds = jsonEncode({
        "ns": "main",
        "db": "main",
        "access": "account", // v3
        "scope": "account", // v2 compat
        "username": username,
        "password": "secure_native_password",
      });

      print('Attempting NATIVE signup for $username...');

      final token = await client.signup(credentialsJson: creds);

      print('✅ SIGNUP SUCCESS! Token received: ${token.substring(0, 20)}...');
      expect(token, isNotEmpty);

      // Verify token usability by authenticating
      await client.authenticate(token: token);
      print('✅ Authentication with new token successful.');
    } catch (e) {
      print('❌ Native Signup Failed: $e');
      rethrow;
    } finally {
      client.close();
    }
  });
}
