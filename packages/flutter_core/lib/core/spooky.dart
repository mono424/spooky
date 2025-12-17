import 'types.dart';
import 'provision.dart'; // Add provision import
import 'services/db.dart';
import 'services/auth.dart';
import 'services/query.dart';
import 'services/logger.dart'; // Add logger import
import 'services/mutation.dart';

export 'types.dart';

class Spooky {
  final SpookyConfig config;
  late final DatabaseService db;
  late final AuthManager auth;
  late final QueryManager queryManager;
  late final MutationManager mutationManager;

  Spooky._(this.config);

  static Future<Spooky> create(SpookyConfig config) async {
    final client = Spooky._(config);
    await client._init();
    return client;
  }

  Future<void> _init() async {
    db = DatabaseService(config);
    await db.init();

    await runProvision(config.database, config.schemaString, db, Logger());

    //auth = AuthManager(db);
    //queryManager = QueryManager(db);
    //mutationManager = MutationManager(db);
  }

  /// Query interface
  QueryReturnType query(String surrealql) {
    // Register query immediately
    final hashFuture = queryManager.register(surrealql);
    return QueryReturnType(queryManager, hashFuture);
  }

  /// Mutation interface
  MutationManager get mutation => mutationManager;

  /// Auth interface
  AuthManager get authentication => auth;

  Future<void> close() async {
    await db.close();
  }
}

class QueryReturnType {
  final QueryManager _manager;
  final Future<int> _hashFuture;

  QueryReturnType(this._manager, this._hashFuture);

  Function() subscribe(Function(dynamic) callback) {
    Function()? innerUnsubscribe;
    bool disposed = false;

    _hashFuture.then((hash) {
      if (!disposed) {
        innerUnsubscribe = _manager.subscribe(hash, callback);
      }
    });

    return () {
      disposed = true;
      if (innerUnsubscribe != null) {
        innerUnsubscribe!();
      }
    };
  }
}
