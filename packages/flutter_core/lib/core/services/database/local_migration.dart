import 'dart:convert';
import 'package:crypto/crypto.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

import './local.dart';

class LocalMigration {
  final LocalDatabaseService local;

  LocalMigration(this.local);

  Future<void> provision(String schemaSurql) async {
    final hash = _sha1(schemaSurql);
    final database = local.getConfig.database;

    // Get the local database client
    final localDb = local.getClient;

    // Start a transaction
    final tx = await localDb.beginTransaction();

    try {
      if (await _isSchemaUpToDate(tx, hash)) {
        print("[Provisioning] Schema is up to date, skipping migration");
        // We still need to commit or cancel/close the transaction?
        // Although if we did nothing, committing is fine.
        await tx.commit();
        return;
      }

      await _recreateDatabase(tx, database);

      final statements = schemaSurql
          .split(";")
          .map((s) => s.trim())
          .where((s) => s.isNotEmpty)
          .toList();

      for (var i = 0; i < statements.length; i++) {
        final statement = statements[i];
        try {
          await tx.query(query: statement);
          print(
            "[Provisioning] (${i + 1}/${statements.length}) Executed: ${statement.substring(0, statement.length > 50 ? 50 : statement.length)}...",
          );
        } catch (e) {
          print(
            "[Provisioning] (${i + 1}/${statements.length}) Error executing statement: $statement",
          );
          rethrow;
        }
      }

      await _createHashRecord(tx, hash);
      await tx.commit();
    } catch (e) {
      print("[Provisioning] Error during provision: $e");
      try {
        await tx.cancel();
      } catch (cancelError) {
        print("[Provisioning] Error cancelling transaction: $cancelError");
      }
      rethrow;
    }
  }

  Future<bool> _isSchemaUpToDate(SurrealTransaction tx, String hash) async {
    try {
      final results = await tx.query(
        query:
            "SELECT hash, created_at FROM _spooky_schema ORDER BY created_at DESC LIMIT 1;",
      );

      if (results.isEmpty) return false;
      final firstResult = results.first;
      if (firstResult.status != "OK" || firstResult.result == null)
        return false;

      final decoded = jsonDecode(firstResult.result!);
      // decoded should be a List of objects for SELECT
      if (decoded is List && decoded.isNotEmpty) {
        final record = decoded[0];
        if (record is Map<String, dynamic>) {
          return record['hash'] == hash;
        }
      }
      return false;
    } catch (error) {
      return false;
    }
  }

  Future<void> _recreateDatabase(SurrealTransaction tx, String database) async {
    await tx.query(
      query:
          """
      USE DB _spooky_temp;
      REMOVE DATABASE $database;
      DEFINE DATABASE $database;
      USE DB $database;
    """,
    );
  }

  Future<void> _createHashRecord(SurrealTransaction tx, String hash) async {
    // Parameter binding is supported in query logic of lib.rs
    final vars = jsonEncode({'hash': hash});
    await tx.query(
      query:
          "UPSERT _spooky_schema SET hash = \$hash, created_at = time::now() WHERE hash = \$hash;",
      vars: vars,
    );
  }

  String _sha1(String input) {
    var bytes = utf8.encode(input);
    var digest = sha1.convert(bytes);
    return digest.toString();
  }
}
