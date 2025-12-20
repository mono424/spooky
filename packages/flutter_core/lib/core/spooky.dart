import 'types.dart';
import 'provision.dart'; // Add provision import
import 'backup/db.dart';
import 'backup/auth.dart';
import 'backup/query.dart';
import 'services/logger.dart'; // Add logger import
import 'backup/mutation.dart';

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
    

    final res = await db.queryLocal('CREATE thread SET author = user:am4v458ret6ocg1kalzl, title = "Hello", content = "Hello"'); 
    print(res);


    //await runProvision(config.database, config.schemaString, db, Logger());

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
