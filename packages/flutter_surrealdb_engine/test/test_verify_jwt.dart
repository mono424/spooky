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

  test('Signup Test with JWT Configured', () async {
    print('Connecting to live server at ws://127.0.0.1:8000/rpc...');

    final client = await SurrealDb.connect(
      mode: StorageMode.remote(url: 'ws://127.0.0.1:8000/rpc'),
    );
    print('Connected.');

    try {
      final username = "jwt_live_user_${DateTime.now().millisecondsSinceEpoch}";
      final creds = jsonEncode({
        "ns": "main",
        "db": "main",
        "access": "account",
        "username": username,
        "password": "secure_password",
      });

      print('Attempting SIGNUP for $username...');
      // We expect this to work now that the server has a signing key.
      // And the schema returns { id, username }, which is safe.
      final token = await client.signup(credentialsJson: creds);
      print('✅ SIGNUP SUCCESS! Token received: ${token.substring(0, 20)}...');
    } catch (e) {
      print('❌ Signup Failed: $e');
      rethrow;
    } finally {
      client.close();
    }
  });
}
