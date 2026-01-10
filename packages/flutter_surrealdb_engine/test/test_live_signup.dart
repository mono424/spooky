import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('Live Server Signup Verification', () async {
    print('Initializing RustLib...');
    await RustLib.init();

    print('Connecting to live server at ws://127.0.0.1:8000/rpc...');
    // Use the generated SurrealDb class
    final client = await SurrealDb.connect(
      mode: const StorageMode.remote(url: 'ws://127.0.0.1:8000/rpc'),
    );

    print('Connected.');

    final username = 'live_test_user_${DateTime.now().millisecondsSinceEpoch}';
    final password = 'password';

    print('Attempting signup for user: $username');

    try {
      final credentials = jsonEncode({
        "username": username,
        "password": password,
        "ns": "main",
        "db": "main",
        "access": "account",
      });

      final token = await client.signup(credentialsJson: credentials);
      print('Signup successful! Token received: $token');

      // Cleanup
      await client.close();
    } catch (e) {
      print('Signup failed: $e');
      if (e.toString().contains('The record access signup query failed')) {
        print('FAILURE CONFIRMED: Server is still running old schema.');
      }
      // Close matching try/catch
      try {
        await client.close();
      } catch (_) {}

      rethrow;
    }
  });
}
