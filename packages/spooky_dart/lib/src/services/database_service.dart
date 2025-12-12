import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/src/rust/lib.dart' as engine;

class DatabaseService {
  bool _isConnected = false;

  bool get isConnected => _isConnected;

  Future<void> connect(String path) async {
    if (_isConnected) return;
    try {
      await engine.connectDb(path: path);
      _isConnected = true;
    } catch (e) {
      throw Exception("Failed to connect to database: $e");
    }
  }

  Future<List<dynamic>> query(
    String query, [
    Map<String, dynamic>? vars,
  ]) async {
    if (!_isConnected) {
      throw Exception("Database not connected");
    }

    // Replace variables in query string (naive implementation)
    // In a real implementation, we should pass vars to the engine if supported.
    // The current engine API only takes a query string.
    String finalQuery = query;
    if (vars != null) {
      vars.forEach((key, value) {
        // Simple string replacement for now.
        // WARNING: This is vulnerable to injection if not handled carefully.
        // Ideally, the Rust engine should support parameterized queries.
        // For now, we assume trusted input or basic escaping.
        String valStr = jsonEncode(value);
        finalQuery = finalQuery.replaceAll("\$$key", valStr);
      });
    }

    final results = await engine.queryDb(query: finalQuery);

    // Process results
    // The engine returns a list of SurrealResult objects
    final processedResults = <dynamic>[];

    for (final res in results) {
      if (res.status != "OK") {
        throw Exception("Query failed: ${res.status}");
      }
      if (res.result != null) {
        try {
          processedResults.add(jsonDecode(res.result!));
        } catch (e) {
          // If result is not JSON, return as is? Or null?
          processedResults.add(res.result);
        }
      } else {
        processedResults.add(null);
      }
    }

    return processedResults;
  }
}
