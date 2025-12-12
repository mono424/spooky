import 'package:query_builder_dart/query_builder_dart.dart';
import 'services/database_service.dart';
import 'services/auth_manager.dart';
import 'services/query_manager.dart';
import 'services/mutation_manager.dart';
import 'services/event_system.dart';

class SpookyConfig {
  final String databasePath;
  final SchemaStructure schema;

  SpookyConfig({required this.databasePath, required this.schema});
}

class SpookyInstance {
  final DatabaseService _db;
  final AuthManager auth;
  final QueryManager _queryManager;
  final MutationManager mutation;
  final EventSystem _events;

  SpookyInstance._(
    this._db,
    this.auth,
    this._queryManager,
    this.mutation,
    this._events,
  );

  static Future<SpookyInstance> initialize(SpookyConfig config) async {
    final db = DatabaseService();
    await db.connect(config.databasePath);

    final events = EventSystem();
    final auth = AuthManager(db, events);
    final queryManager = QueryManager(db, events, config.schema);
    final mutation = MutationManager(db, events);

    return SpookyInstance._(db, auth, queryManager, mutation, events);
  }

  /// Get the event stream
  Stream<SpookyEvent> get onEvent => _events.onEvent;

  /// Register a live query
  Future<String> query(String tableName, QueryOptions options) {
    return _queryManager.registerQuery(tableName, options);
  }

  /// Unregister a live query
  Future<void> unregisterQuery(String incantationId) {
    return _queryManager.unregisterQuery(incantationId);
  }

  /// Dispose the instance
  void dispose() {
    _events.dispose();
  }
}
