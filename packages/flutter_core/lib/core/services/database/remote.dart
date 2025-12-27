import 'dart:convert';

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
        // Rethrow the error so it can be seen in the UI logs for debugging
        throw Exception(
          'Failed to connect to remote SurrealDB at ${config.endpoint}: $e',
        );
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
        "CREATE ONLY user SET username = \$username, password = crypto::argon2::generate(\$password);";
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
}
