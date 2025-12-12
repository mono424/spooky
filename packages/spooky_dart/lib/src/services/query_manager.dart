import 'package:query_builder_dart/query_builder_dart.dart';
import 'package:uuid/uuid.dart';
import 'database_service.dart';
import 'event_system.dart';

class QueryManager {
  final DatabaseService _db;
  final EventSystem _events;
  final SchemaStructure _schema;
  final _uuid = Uuid();

  // Active live queries
  final Map<String, String> _activeQueries = {}; // queryId -> incantationId

  QueryManager(this._db, this._events, this._schema);

  /// Register a live query
  Future<String> registerQuery(String tableName, QueryOptions options) async {
    // 1. Build the query string
    final builder = QueryBuilder(_schema, tableName, options);
    final queryInfo = builder.build();
    final surrealQl = queryInfo.query;

    // 2. Generate a unique ID for this query instance (Incantation ID)
    // In the TS version, this is a hash. Here we use a UUID for simplicity or hash the query.
    // Let's use a UUID for the registration ID.
    final incantationId = _uuid.v4();

    // 3. Register in _spooky_incantation table (Shadow Graph)
    // We need to insert a record into _spooky_incantation
    // fields: Id, SurrealQL, etc.
    final createQuery =
        "CREATE _spooky_incantation SET Id = '$incantationId', SurrealQL = \"$surrealQl\", TTL = '1h'";

    try {
      await _db.query(createQuery);

      // 4. Register lookups (reverse index)
      // We need to parse the WHERE clause to find what we are filtering on.
      // This is complex. For now, we just register a basic lookup for the table.
      final lookupId = _uuid.v4();
      final createLookup =
          "CREATE _spooky_incantation_lookup SET IncantationId = '$incantationId', Table = '$tableName'";
      await _db.query(createLookup);

      _activeQueries[incantationId] = incantationId;
      return incantationId;
    } catch (e) {
      _events.emit(SpookyEventType.error, "Failed to register query: $e");
      rethrow;
    }
  }

  /// Unregister a live query
  Future<void> unregisterQuery(String incantationId) async {
    if (!_activeQueries.containsKey(incantationId)) return;

    final deleteQuery =
        "DELETE _spooky_incantation WHERE Id = '$incantationId'";
    try {
      await _db.query(deleteQuery);
      _activeQueries.remove(incantationId);
    } catch (e) {
      _events.emit(SpookyEventType.error, "Failed to unregister query: $e");
    }
  }
}
