import 'dart:convert';

import '../../types.dart';
import '../database/local_database_service.dart';

/// [PersistenceClient] backed by the local sqlite `_00_kv` table, so values
/// (stream-processor circuit state, auth token) survive process restarts.
///
/// Values are JSON-encoded, so arbitrary JSON-safe types round-trip; string
/// values come back as plain strings.
class SqlitePersistenceClient implements PersistenceClient {
  SqlitePersistenceClient(this._db);

  final LocalDatabaseService _db;

  @override
  Future<void> set(String key, dynamic value) async {
    _db.kvSet(key, jsonEncode(value));
  }

  @override
  Future<T?> get<T>(String key) async {
    final raw = _db.kvGet(key);
    if (raw == null) return null;
    return jsonDecode(raw) as T?;
  }

  @override
  Future<void> remove(String key) async {
    _db.kvRemove(key);
  }
}
