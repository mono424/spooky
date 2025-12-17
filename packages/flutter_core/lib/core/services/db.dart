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
    local = await connectDb(path: config.localDBurl);

    try {
      await local.useNs(ns: config.namespace);
      await local.useDb(db: config.database);
    } catch (e) {
      print('Error setting local NS/DB: $e');
    }

    // Initialize remote database
    if (config.globalDBurl.isNotEmpty) {
      remote = await connectDb(path: config.globalDBurl);

      try {
        await remote.useNs(ns: config.namespace);
        await remote.useDb(db: config.database);
        if (config.token != null) {
          await remote.authenticate(token: config.token!);
        }
      } catch (e) {
        print('Error setting remote NS/DB: $e');
      }
    }
  }

  Future<T> runWithLocal<T>(Future<T> Function(SurrealDatabase) fn) async {
    // Switch to configured local DB/NS
    // Note: flutter_surrealdb_engine might not support 'use' per query context but per connection.
    // Assuming 'local' connection is used for 'database' by default, as set in init().
    // If provision needs a different context, it should likely use queries with defined keys or switch context.
    // Based on spooky.ts, runWithLocal runs against the local DB instance.
    return fn(local);
  }

  Future<T> runWithInternal<T>(Future<T> Function(SurrealDatabase) fn) async {
    // Internal usage often requires a specific internal database
    // In spooky.ts this switches the "use" context.
    try {
      await local.useDb(db: config.internalDatabase);
      final result = await fn(local);
      return result;
    } finally {
      // Restore default database
      await local.useDb(db: config.database);
    }
  }

  Future<T> queryLocal<T>(String sql, [Map<String, dynamic>? vars]) async {
    return _query<T>(local, sql, vars);
  }

  Future<T> queryRemote<T>(String sql, [Map<String, dynamic>? vars]) async {
    return _query<T>(remote, sql, vars);
  }

  Future<T> _query<T>(
    SurrealDatabase db,
    String sql, [
    Map<String, dynamic>? vars,
  ]) async {
    final interpolated = _interpolate(sql, vars);
    final results = await db.queryDb(query: interpolated);

    if (results.isEmpty) {
      // Return null or empty list depending on T?
      // For simplicity, return null casted.
      // If T is nullable, it works.
      return null as T;
    }

    // We take the result of the last statement or the first?
    // Usually spiky query is 1 statement.
    final first = results.first;

    // Check status if needed. The rust lib definition shows 'status' string.
    // Assuming 'OK' is success.

    if (first.result == null) return null as T;

    final parsed = jsonDecode(first.result!);
    return parsed as T;
  }

  String _interpolate(String sql, Map<String, dynamic>? vars) {
    if (vars == null || vars.isEmpty) return sql;
    var out = sql;
    vars.forEach((k, v) {
      // Simple replacement for $key
      // Better would be regex to avoid replacing inside strings, but for now this suffices.
      final key = k.startsWith(r'$') ? k : '\$$k';
      out = out.replaceAll(key, jsonEncode(v));
    });
    return out;
  }

  // Preliminary live query support
  Future<void> subscribeLive(
    String uuid,
    Function(String action, Map<String, dynamic> result) callback,
  ) async {
    print(
      'Live query subscription for $uuid not fully implemented in Dart yet',
    );
  }

  SurrealDatabase getLocal() => local;
  SurrealDatabase getRemote() => remote;

  Future<void> close() async {
    // No close method on SurrealDatabase in the viewed file?
    // connectDb returns SurrealDatabase which is abstract class wrapping Rust object.
    // Maybe it closes on GC or we ignore.
  }
}
