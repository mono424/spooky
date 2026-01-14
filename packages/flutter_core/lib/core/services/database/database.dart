import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

abstract class AbstractDatabaseService {
  final SurrealDb? client;
  // Logger support can be added if we have a Logger in flutter_core, currently omitting or simple print
  // final Logger logger;

  AbstractDatabaseService(this.client);

  SurrealDb get getClient => client!;

  Future<void> connect(); // Matches core's abstract connect()

  // Alias for legacy support if needed, or we implement connect in LocalDatabaseService
  Future<void> init() => connect();

  Future<void> close() async {
    client?.close();
  }

  /// Execute a query with serialized execution (though Dart is single isolate,
  /// we might not need the queue if engine handles it, but core has it).
  /// For now, we match the signature: returns Generic T.
  Future<T> query<T>(String sql, {Object? vars}) async {
    return _executeQuery<T>(
      sql,
      vars,
      (s, v) => client!.query(sql: s, vars: v),
    );
  }

  Future<T> queryTyped<T>(String sql, {Object? vars}) async {
    return _executeQuery<T>(
      sql,
      vars,
      (s, v) => client!.queryTyped(sql: s, vars: v),
    );
  }

  Future<T> _executeQuery<T>(
    String sql,
    Object? vars,
    Future<String> Function(String, String?) fetcher,
  ) async {
    if (client == null) throw Exception("Client not initialized");
    final result = await fetcher(sql, vars != null ? jsonEncode(vars) : null);

    if (T == String) {
      return result as T;
    }
    // Attempt decode
    try {
      final decoded = jsonDecode(result);
      return decoded as T;
    } catch (_) {
      return result as T;
    }
  }

  /// Exports the database to the specified path.
  Future<void> export(String path) async {
    await client?.export_(path: path);
  }
}
