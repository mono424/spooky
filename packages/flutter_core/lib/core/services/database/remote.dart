import 'dart:convert';
import 'dart:async';

import './database.dart';
import '../../types.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

class RemoteDatabaseService extends AbstractDatabaseService {
  final DatabaseConfig config;

  RemoteDatabaseService._(super.client, this.config);

  static Future<RemoteDatabaseService> connect(DatabaseConfig config) async {
    SurrealDb? client;
    if (config.endpoint != null) {
      try {
        client = await SurrealDb.connect(
          mode: StorageMode.remote(url: config.endpoint!),
        );
      } catch (e) {
        // Log the error but don't crash. Return service in "offline" mode (client = null).
        print(
          'Warning: Failed to connect to remote SurrealDB at ${config.endpoint}: $e',
        );
        // We do NOT rethrow here. The app will proceed with client = null.
      }
    }
    return RemoteDatabaseService._(client, config);
  }

  DatabaseConfig get getConfig => config;

  @override
  Future<void> init() async {
    if (client == null) return;
    try {
      await client!.useDb(ns: config.namespace, db: config.database);
      if (config.token != null) {
        await client!.authenticate(token: config.token!);
      }
    } catch (e) {
      throw Exception('Error setting remote NS/DB: $e');
    }
  }

  Future<String> manualSignup({
    required String username,
    required String password,
    required String namespace,
    required String database,
  }) async {
    // 1. Create user via query (Requires public permissions or current auth)
    // Since schema has "FOR create WHERE true", this works without auth.
    final query =
        r'''CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password);''';
    // vars is now named parameter type object!
    final response = await this.query(
      query,
      vars: {"username": username, "password": password},
    );

    // Check if creation failed
    if (response.contains("error") || response.contains("Error")) {
      throw Exception("Failed to create user: $response");
    }

    // 2. Signin to get the token
    final credentials = jsonEncode({
      "ns": namespace,
      "db": database,
      "access": "account",
      "username": username,
      "password": password,
    });
    return await client!.signin(creds: credentials);
  }

  /// Subscribe to a Live Query on the given table.
  /// Returns a [StreamSubscription] that must be managed/cancelled by the caller.
  StreamSubscription? subscribeLive({
    required String tableName,
    required void Function(String action, Map<String, dynamic> result) callback,
  }) {
    if (client == null) {
      print(
        "[RemoteDatabaseService] Warning: subscribeLive called but client is null",
      );
      return null;
    }

    // We use snapshot=false to match the 'diff' behavior of the TS implementation
    return client!.liveQuery(tableName: tableName, snapshot: false).listen(
      (event) {
        try {
          // Decode the result JSON
          final result = jsonDecode(event.result);
          if (result is Map<String, dynamic>) {
            // Map action enum to string (CREATE, UPDATE, DELETE)
            // TS expects uppercase: 'UPDATE', 'CREATE', etc.
            final actionStr = event.action.name.toUpperCase();
            callback(actionStr, result);
          }
        } catch (e) {
          print("[RemoteDatabaseService] Error parsing live query result: $e");
        }
      },
      onError: (e) =>
          print("[RemoteDatabaseService] Live Query Stream Error: $e"),
    );
  }

  Future<void> authenticate(String token) async {
    if (client == null) throw Exception("Remote client unavailable");
    await client!.authenticate(token: token);
  }

  Future<void> invalidate() async {
    if (client == null) return;
    await client!.invalidate();
  }
}
