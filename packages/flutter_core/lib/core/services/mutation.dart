import 'db.dart';

class MutationManager {
  final DatabaseService db;

  MutationManager(this.db);

  Future<T> create<T>(String table, Map<String, dynamic> data) async {
    // Perform creation on local DB
    // Assuming queryLocal returns a List of results
    // If the data is generic map, we cast it to T
    final result = await db.queryLocal(
      r'CREATE type::table($table) CONTENT $data',
      {'table': table, 'data': data},
    );

    // logic to extract T from result (assuming result is [T] or T)
    // simplifying for now
    final created = (result is List ? result.first : result) as T;

    // Sync to remote
    db
        .queryRemote(r'CREATE type::table($table) CONTENT $data', {
          'table': table,
          'data': data,
        })
        .catchError((e) {
          print('Error sinking create to remote: $e');
        });

    return created;
  }

  Future<T> update<T>(
    String table,
    String id,
    Map<String, dynamic> data,
  ) async {
    final rid = '$table:$id'; // Simple string manipulation for RecordId

    final result = await db.queryLocal(r'UPDATE $id MERGE $data', {
      'id': rid,
      'data': data,
    });

    final updated = (result is List ? result.first : result) as T;

    db
        .queryRemote(r'UPDATE $id MERGE $data', {'id': rid, 'data': data})
        .catchError((e) {
          print('Error sinking update to remote: $e');
        });

    return updated;
  }

  Future<void> delete(String table, String id) async {
    final rid = '$table:$id';

    // Using query because we usually don't have direct .delete() exposed in DatabaseService wrapper
    // unless we use getLocal().delete(rid), but let's stick to query for consistency with params
    await db.queryLocal(r'DELETE $id', {'id': rid});

    db.queryRemote(r'DELETE $id', {'id': rid}).catchError((e) {
      print('Error sinking delete to remote: $e');
    });
  }
}
