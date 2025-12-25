import 'dart:convert';
import 'package:crypto/crypto.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

import './surreal_decoder.dart';
import './local.dart';

class LocalMigration {
  final LocalDatabaseService local;

  LocalMigration(this.local);

  Future<void> provision(String schemaSurql) async {
    final hash = _sha1(schemaSurql);
    final database = local.getConfig.database;
    final localDb = local.getClient;

    // Start a transaction
    await localDb.queryBegin();

    try {
      if (await _isSchemaUpToDate(localDb, hash)) {
        print("[Provisioning] Schema is up to date, skipping migration");
        await localDb.queryCommit();
        return;
      }

      print("[Provisioning] Schema is not up to date, running migration");

      await _recreateDatabase(localDb, database);

      final statements = _splitSurqlStatements(schemaSurql);

      for (var i = 0; i < statements.length; i++) {
        final statement = statements[i];
        try {
          await localDb.query(sql: statement, vars: "{}");
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

      await _createHashRecord(localDb, hash);
      await localDb.queryCommit();
    } catch (e) {
      print("[Provisioning] Error during provision: $e");
      try {
        await localDb.queryCancel();
      } catch (cancelError) {
        print("[Provisioning] Error cancelling transaction: $cancelError");
      }
      rethrow;
    }
  }

  Future<bool> _isSchemaUpToDate(SurrealDb db, String hash) async {
    try {
      final jsonResult = await db.query(
        sql:
            "SELECT hash, created_at FROM _spooky_schema ORDER BY created_at DESC LIMIT 1;",
        vars: "{}",
      );

      // Parse the JSON result directly since new query returns String (JSON)
      // The engine result is typically a list of query results.
      // Assuming result is "[{...}]" or similar structure.
      // But query returns "String", which is the raw JSON response from SurrealDB.

      final List<dynamic> results = jsonDecode(jsonResult);
      if (results.isEmpty) return false;

      final firstResult = results[0];
      // Check status/result structure if wrapper exists
      // Engine `query` returns the JSON directly from the Rust bridge.
      // It mimics the structure: [{ status: 'OK', result: [ ... ], time: ... }]

      if (firstResult['status'] != "OK" || firstResult['result'] == null) {
        return false;
      }

      final resArray = SurrealDecoder.unwrap(
        firstResult['result'],
      ); // Unwrap the 'result' field

      if (resArray is SurrealArray) {
        if (resArray.items.isNotEmpty) {
          final firstElement = resArray.items.first;
          if (firstElement is SurrealObject) {
            final remoteHashVal = firstElement.fields['hash'];
            final remoteHash = remoteHashVal?.toString();

            if (remoteHash != null && remoteHash == hash) {
              return true;
            }
          }
        }
      }
      return false;
    } catch (error) {
      return false;
    }
  }

  Future<void> _recreateDatabase(SurrealDb db, String database) async {
    await db.query(
      sql:
          """
      USE DB _spooky_temp;
      REMOVE DATABASE $database;
      DEFINE DATABASE $database;
      USE DB $database;
    """,
      vars: "{}",
    );
  }

  Future<void> _createHashRecord(SurrealDb db, String hash) async {
    // Parameter binding is supported in query logic of lib.rs
    final vars = jsonEncode({'hash': hash});
    await db.query(
      sql:
          "UPSERT _spooky_schema SET hash = \$hash, created_at = time::now() WHERE hash = \$hash;",
      vars: vars,
    );
  }

  String _sha1(String input) {
    var bytes = utf8.encode(input);
    var digest = sha1.convert(bytes);
    return digest.toString();
  }

  /// Splits a SurQL schema string into individual statements, respecting
  /// blocks {...} and quotes '...' or "...".
  List<String> _splitSurqlStatements(String schema) {
    final statements = <String>[];
    var buffer = StringBuffer();

    var inSingleQuote = false;
    var inDoubleQuote = false;
    var braceDepth = 0;

    for (var i = 0; i < schema.length; i++) {
      final char = schema[i];
      final prevChar = i > 0 ? schema[i - 1] : null;

      // Handle quotes (ignoring escaped quotes)
      if (char == "'" && !inDoubleQuote && prevChar != '\\') {
        inSingleQuote = !inSingleQuote;
      } else if (char == '"' && !inSingleQuote && prevChar != '\\') {
        inDoubleQuote = !inDoubleQuote;
      }

      // Handle braces (only if not in quotes)
      if (!inSingleQuote && !inDoubleQuote) {
        if (char == '{') {
          braceDepth++;
        } else if (char == '}') {
          braceDepth--;
        }
      }

      // Handle split point
      if (char == ';' && !inSingleQuote && !inDoubleQuote && braceDepth == 0) {
        final stmt = buffer.toString().trim();
        if (stmt.isNotEmpty) {
          statements.add(stmt);
        }
        buffer.clear();
      } else {
        buffer.write(char);
      }
    }

    // Add any remaining content
    final remaining = buffer.toString().trim();
    if (remaining.isNotEmpty) {
      statements.add(remaining);
    }

    return statements;
  }
}
