import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import '../types.dart';

class DatabaseService {
  late SurrealDatabase local;
  late SurrealDatabase remote;
  final SpookyConfig config;

  DatabaseService(this.config);

  Future<void> init() async {
    // Initialize FFI bindings
    await RustLib.init();

    // Initialize local database
    local = await connectDb(
      path: config.database.path,
    );

    try {
      await local.useNs(ns: config.database.namespace);
      await local.useDb(db: config.database.database);
    } catch (e) {
      throw Exception('Error setting local NS/DB: $e');
    }

    // Initialize remote database
    if (config.database.endpoint != null) {
      remote = await connectDb(path: config.database.endpoint!);

      try {
        await remote.useNs(ns: config.database.namespace);
        await remote.useDb(db: config.database.database);
        if (config.database.token != null) {
          await remote.authenticate(token: config.database.token!);
        }
      } catch (e) {
        print('Error setting remote NS/DB: $e');
      }
    }
  }

  Future<T?> queryLocal<T>(String sql, [Map<String, dynamic>? vars]) async {
    final varsJson = vars != null ? jsonEncode(vars) : null;
    final results = await local.queryDb(query: sql, vars: varsJson);
    return results;
    //return _parseResult<T>(results);
  }

  Future<T?> queryRemote<T>(String sql, [Map<String, dynamic>? vars]) async {
    final varsJson = vars != null ? jsonEncode(vars) : null;
    final results = await remote.queryDb(query: sql, vars: varsJson);
    return _parseResult<T>(results);
  }

  SurrealDatabase getLocal() {
    return local;
  }

  SurrealDatabase getRemote() {
    return remote;
  }

  Future<void> close() async {
    // Invalidate sessions
    await local.invalidate();
    if (config.database.endpoint != null) {
      await remote.invalidate();
    }
  }

  T? _parseResult<T>(List<SurrealResult> results) {
    if (results.isEmpty) return null;
    final firstResult = results.first;
    if (firstResult.status != 'OK') {
      throw Exception(
        'Query failed: ${firstResult.status} - ${firstResult.result}',
      );
    }
    if (firstResult.result == null) return null;

    final decoded = jsonDecode(firstResult.result!);
    // Rust serialization puts results in specific structure depending on query type?
    // Based on integration test: {"Array": [...]} or raw value.
    // However, usually we want the direct value.
    // If T is List, we expect the array content.
    // Let's return decoded as T for now, user might need to cast or we refine parsing.

    return decoded as T;
  }
}
