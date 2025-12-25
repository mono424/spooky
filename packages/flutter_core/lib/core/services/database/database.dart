import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

abstract class AbstractDatabaseService {
  final SurrealDb? client;

  AbstractDatabaseService(this.client);

  SurrealDb get getClient => client!;

  Future<void> init();

  Future<void> close() async {
    client?.close();
  }

  /// Execute a query with optional variables.
  /// [sql] is required and positional.
  /// [vars] is optional, named, and can be any serializable Object (e.g. Map).
  Future<String> query(String sql, {Object? vars}) async {
    if (client == null) throw Exception("Client not initialized");
    return client!.query(
      sql: sql,
      vars: vars != null ? jsonEncode(vars) : null,
    );
  }
}
