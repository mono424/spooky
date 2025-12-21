import 'types.dart';
export 'types.dart';

class SpookyClient {
  final SpookyConfig config;

  SpookyClient._({required this.config});

  static Future<Spooky> create(SpookyConfig config) async {
    final client = SpookyClient._({config: config});
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
