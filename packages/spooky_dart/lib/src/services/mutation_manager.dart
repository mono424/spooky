import 'database_service.dart';
import 'event_system.dart';

class MutationManager {
  final DatabaseService _db;
  final EventSystem _events;

  MutationManager(this._db, this._events);

  Future<dynamic> create(String table, Map<String, dynamic> data) async {
    // Construct CREATE query
    // Ideally use parameterized query
    // Naive implementation:
    final fields = data.keys.join(", ");
    // Values need to be JSON encoded
    // This is tricky without a proper query builder for INSERT/CREATE
    // Let's use CONTENT clause which is safer if supported
    // CREATE type::table($table) CONTENT $data

    final query = "CREATE type::table(\$table) CONTENT \$data";
    final vars = {'table': table, 'data': data};

    try {
      final res = await _db.query(query, vars);
      if (res.isNotEmpty) {
        _events.emit(SpookyEventType.mutation, {
          'type': 'create',
          'table': table,
          'id': res[0]['id'],
        });
        return res[0];
      }
      return null;
    } catch (e) {
      _events.emit(SpookyEventType.error, "Create failed: $e");
      rethrow;
    }
  }

  Future<dynamic> update(String id, Map<String, dynamic> data) async {
    final query = "UPDATE \$id MERGE \$data";
    final vars = {'id': id, 'data': data};

    try {
      final res = await _db.query(query, vars);
      if (res.isNotEmpty) {
        _events.emit(SpookyEventType.mutation, {'type': 'update', 'id': id});
        return res[0];
      }
      return null;
    } catch (e) {
      _events.emit(SpookyEventType.error, "Update failed: $e");
      rethrow;
    }
  }

  Future<void> delete(String id) async {
    final query = "DELETE \$id";
    final vars = {'id': id};

    try {
      await _db.query(query, vars);
      _events.emit(SpookyEventType.mutation, {'type': 'delete', 'id': id});
    } catch (e) {
      _events.emit(SpookyEventType.error, "Delete failed: $e");
      rethrow;
    }
  }
}
