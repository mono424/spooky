import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/codegen/dart_emitter.dart';
import 'package:spooky_core/src/codegen/schema_parser.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/modules/data/relation_resolver.dart';
import 'package:spooky_core/src/modules/relationships.dart';
import 'package:spooky_core/src/modules/sync/sync.dart';
import 'package:spooky_core/src/services/database/remote_database_service.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// Serves the two `_00_list_ref` shapes (primary window vs subquery children)
/// and the record bodies behind them, recording which ids were fetched.
class _ChildRemote implements RemoteSurrealClient {
  final Map<String, Map<String, dynamic>> records = {};
  final List<String> queries = [];
  final List<String> fetchedIds = [];
  List<Map<String, dynamic>> primaryListRef = [];
  List<Map<String, dynamic>> subqueryListRef = [];

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    queries.add(sql);
    if (sql.contains('parent IS NOT NONE')) return [subqueryListRef];
    if (sql.contains('parent IS NONE')) return [primaryListRef];
    if (sql.contains(r'$idsToFetch')) {
      final ids = (vars?['idsToFetch'] as List?) ?? const [];
      fetchedIds.addAll(ids.map((id) => id.toString()));
      return [
        ids
            .map((id) => records[id.toString()])
            .where((r) => r != null)
            .toList(),
      ];
    }
    if (sql.contains(r'$ids')) return [<dynamic>[]];
    return [null];
  }

  @override
  Future<void> connect(String endpoint) async {}
  @override
  Future<void> use(
      {required String namespace, required String database}) async {}
  @override
  Future<dynamic> authenticate(String token) async => null;
  @override
  Future<dynamic> signin(Map<String, dynamic> p) async => {'access': 't'};
  @override
  Future<dynamic> signup(Map<String, dynamic> p) async => {'access': 't'};
  @override
  Future<void> invalidate() async {}
  @override
  Future<(String, Stream<LiveMessage>)> live(String sql,
          [Map<String, dynamic>? vars]) async =>
      ('l', const Stream<LiveMessage>.empty());
  @override
  Future<void> kill(String liveId) async {}
  @override
  Stream<void> get onConnected => const Stream.empty();
  @override
  Stream<void> get onDisconnected => const Stream.empty();
  @override
  Future<void> close() async {}
}

Future<void> _tick() => Future<void>.delayed(const Duration(milliseconds: 20));

/// `.related()` end to end: relationships derived from the schema, the emitted
/// correlated subqueries, and the local-cache resolver that attaches joined rows
/// (the Dart stand-in for SurrealQL's nested projections).
void main() {
  final logger = SpookyLogger.root('test');

  const schemaSurql = '''
DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD title ON thread TYPE string;
DEFINE FIELD author ON thread TYPE record<user>;
DEFINE TABLE comment SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD body ON comment TYPE string;
DEFINE FIELD score ON comment TYPE int;
DEFINE FIELD thread ON comment TYPE record<thread>;
DEFINE FIELD author ON comment TYPE record<user>;
DEFINE TABLE user SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD name ON user TYPE string;
''';

  /// The runtime schema map a generated client ships, including relationships.
  final schema = <String, dynamic>{
    'thread': {
      'columns': {
        'title': const ColumnSchema(type: 'string'),
        'author': const ColumnSchema(type: 'record', recordId: true),
      },
    },
    'comment': {
      'columns': {
        'body': const ColumnSchema(type: 'string'),
        'score': const ColumnSchema(type: 'int'),
        'thread': const ColumnSchema(type: 'record', recordId: true),
        'author': const ColumnSchema(type: 'record', recordId: true),
      },
    },
    'user': {
      'columns': {'name': const ColumnSchema(type: 'string')},
    },
    'relationships': [
      {'from': 'thread', 'field': 'author', 'to': 'user', 'cardinality': 'one'},
      {'from': 'comment', 'field': 'thread', 'to': 'thread', 'cardinality': 'one'},
      {'from': 'comment', 'field': 'author', 'to': 'user', 'cardinality': 'one'},
      {'from': 'user', 'field': 'threads', 'to': 'thread', 'cardinality': 'many'},
      {'from': 'thread', 'field': 'comments', 'to': 'comment', 'cardinality': 'many'},
      {'from': 'user', 'field': 'comments', 'to': 'comment', 'cardinality': 'many'},
    ],
  };

  group('deriveRelationships', () {
    late List<SchemaRelationship> rels;
    setUp(() {
      rels = deriveRelationships(parseSchema(schemaSurql));
    });

    SchemaRelationship? find(String from, String field) =>
        findRelationship(rels, from, field);

    test('a record<x> field becomes a forward one-relation', () {
      final rel = find('thread', 'author');
      expect(rel, isNotNull);
      expect(rel!.to, 'user');
      expect(rel.cardinality, 'one');
      expect(rel.foreignKeyField, 'author',
          reason: 'a one-relation reads the parent field of the same name');
    });

    test('each forward relation adds a pluralized reverse many-relation', () {
      final rel = find('thread', 'comments');
      expect(rel, isNotNull);
      expect(rel!.to, 'comment');
      expect(rel.cardinality, 'many');
      expect(rel.foreignKeyField, 'thread',
          reason: 'a many-relation matches the child field named after the parent');
      expect(find('user', 'threads')?.to, 'thread');
      expect(find('user', 'comments')?.to, 'comment');
    });

    test('a record field pointing at an unknown table is skipped', () {
      final tables = parseSchema('''
DEFINE TABLE thread SCHEMAFULL;
DEFINE FIELD owner ON thread TYPE record<ghost>;
''');
      expect(deriveRelationships(tables), isEmpty);
    });

    test('an explicit field wins over the derived reverse name', () {
      // `user` already declares a `threads` field, so the reverse relation must
      // not shadow it.
      final tables = parseSchema('''
DEFINE TABLE thread SCHEMAFULL;
DEFINE FIELD author ON thread TYPE record<user>;
DEFINE TABLE user SCHEMAFULL;
DEFINE FIELD threads ON user TYPE string;
''');
      final derived = deriveRelationships(tables);
      expect(findRelationship(derived, 'user', 'threads'), isNull);
      expect(findRelationship(derived, 'thread', 'author'), isNotNull);
    });

    test('pluralization matches the CLI rules', () {
      expect(pluralizeTableName('user'), 'users');
      expect(pluralizeTableName('person'), 'people');
      expect(pluralizeTableName('child'), 'children');
      expect(pluralizeTableName('mouse'), 'mice');
      expect(pluralizeTableName('thread'), 'threads');
      expect(pluralizeTableName('class'), 'classes');
      expect(pluralizeTableName('match'), 'matches');
      expect(pluralizeTableName('box'), 'boxes');
      expect(pluralizeTableName('category'), 'categories');
      expect(pluralizeTableName('play'), 'plays');
      expect(pluralizeTableName('shelf'), 'shelves');
      expect(pluralizeTableName('life'), 'lives');
    });
  });

  group('subquery emission', () {
    QueryBuilder builder(String table) =>
        QueryBuilder(table, schema: schema, logger: logger);

    test('a one-relation correlates on the parent key and takes the first row',
        () {
      final (sql, _) = builder('thread').related('author').build();
      expect(
        sql,
        'SELECT *, (SELECT * FROM user WHERE id=\$parent.author LIMIT 1)[0] AS author FROM thread;',
      );
    });

    test('a many-relation correlates on the child back-reference', () {
      final (sql, _) = builder('thread').related('comments').build();
      expect(
        sql,
        'SELECT *, (SELECT * FROM comment WHERE thread=\$parent.id) AS comments FROM thread;',
      );
    });

    test('a modifier shapes select / where / orderBy / limit', () {
      final (sql, _) = builder('thread').related(
        'comments',
        (c) => c
            .select(['id', 'body'])
            .where({'score': 5})
            .orderBy('score', 'DESC')
            .limit(3),
      ).build();
      expect(
        sql,
        'SELECT *, (SELECT id, body FROM comment WHERE thread=\$parent.id AND score = 5 ORDER BY score DESC LIMIT 3) AS comments FROM thread;',
      );
    });

    test('a record-id literal in a sub-where is not quoted', () {
      final (sql, _) = builder('thread')
          .related('comments', (c) => c.where({'author': 'user:u1'}))
          .build();
      expect(sql, contains('AND author = user:u1'));
      expect(sql, isNot(contains('"user:u1"')));
    });

    test('nested relations nest their subqueries', () {
      final (sql, _) = builder('thread')
          .related('comments', (c) => c.related('author'))
          .build();
      expect(
        sql,
        'SELECT *, (SELECT *, (SELECT * FROM user WHERE id=\$parent.author LIMIT 1)[0] AS author '
        'FROM comment WHERE thread=\$parent.id) AS comments FROM thread;',
      );
    });

    test('relations compose with the parent where / order / window', () {
      final (sql, vars) = builder('thread')
          .related('author')
          .where({'title': 'x'})
          .orderBy('title')
          .limit(2)
          .offset(2)
          .build();
      expect(sql, startsWith('SELECT *, (SELECT * FROM user'));
      expect(sql, contains('FROM thread WHERE title = \$title'));
      expect(sql, endsWith('ORDER BY title ASC LIMIT 2 START 2;'));
      expect(vars, {'title': 'x'});
    });

    test('an unknown relation is skipped, not fatal', () {
      final (sql, _) =
          builder('thread').related('nope').related('author').build();
      expect(sql, isNot(contains('nope')));
      expect(sql, contains('AS author'),
          reason: 'a sibling relation must survive an unknown one');
    });

    test('a repeated relation is added once', () {
      final b = builder('thread').related('author').related('author');
      expect(b.relations, hasLength(1));
    });

    test('a schema without relationships skips every relation', () {
      final (sql, _) = QueryBuilder('thread', schema: const {}, logger: logger)
          .related('author')
          .build();
      expect(sql, 'SELECT * FROM thread;');
    });
  });

  group('resolver', () {
    late LocalDatabaseService local;
    late LocalRelationFetcher fetcher;

    setUp(() {
      local = LocalDatabaseService.open(logger)..provision();
      fetcher = LocalRelationFetcher(local);
      local.create('user:u1', {'name': 'Ada'});
      local.create('user:u2', {'name': 'Linus'});
      local.create('thread:t1', {'title': 'first', 'author': 'user:u1'});
      local.create('thread:t2', {'title': 'second', 'author': 'user:u2'});
      local.create('thread:t3', {'title': 'orphan'});
      for (final (id, thread, score, author) in [
        ('comment:c1', 'thread:t1', 3, 'user:u1'),
        ('comment:c2', 'thread:t1', 1, 'user:u2'),
        ('comment:c3', 'thread:t1', 2, 'user:u1'),
        ('comment:c4', 'thread:t2', 9, 'user:u2'),
      ]) {
        local.create(id, {
          'body': id,
          'thread': thread,
          'score': score,
          'author': author,
        });
      }
    });
    tearDown(() => local.close());

    RelationPlan plan(
      String alias,
      String table,
      String cardinality,
      String fk, {
      Map<String, Object?> where = const {},
      List<(String, String)> orderBy = const [],
      int? limit,
      List<RelationPlan> relations = const [],
    }) =>
        RelationPlan(
          alias: alias,
          table: table,
          cardinality: cardinality,
          foreignKeyField: fk,
          where: where,
          orderBy: orderBy,
          limit: limit,
          relations: relations,
        );

    List<Map<String, dynamic>> threads([List<String> ids = const ['thread:t1']]) =>
        [for (final id in ids) {...local.getById(id)!}];

    test('a one-relation attaches the single row', () {
      final rows = threads();
      resolveRelations(rows, [plan('author', 'user', 'one', 'author')], fetcher);
      expect((rows.single['author'] as Map)['name'], 'Ada');
    });

    test('a one-relation with no foreign key attaches null', () {
      final rows = threads(['thread:t3']);
      resolveRelations(rows, [plan('author', 'user', 'one', 'author')], fetcher);
      expect(rows.single['author'], isNull);
    });

    test('a many-relation attaches the matching children only', () {
      final rows = threads(['thread:t1', 'thread:t2']);
      resolveRelations(
          rows, [plan('comments', 'comment', 'many', 'thread')], fetcher);
      expect((rows[0]['comments'] as List).map((c) => c['id']),
          containsAll(['comment:c1', 'comment:c2', 'comment:c3']));
      expect((rows[1]['comments'] as List).map((c) => c['id']),
          ['comment:c4']);
    });

    test('a many-relation with no children attaches an empty list', () {
      final rows = threads(['thread:t3']);
      resolveRelations(
          rows, [plan('comments', 'comment', 'many', 'thread')], fetcher);
      expect(rows.single['comments'], isEmpty);
    });

    test('order and limit apply PER parent', () {
      final rows = threads(['thread:t1', 'thread:t2']);
      resolveRelations(
        rows,
        [
          plan('comments', 'comment', 'many', 'thread',
              orderBy: [('score', 'desc')], limit: 2)
        ],
        fetcher,
      );
      // Top 2 of thread:t1 by score, not the global top 2 (which would be c4).
      expect((rows[0]['comments'] as List).map((c) => c['id']),
          ['comment:c1', 'comment:c3']);
      expect((rows[1]['comments'] as List).map((c) => c['id']),
          ['comment:c4']);
    });

    test('a sub-where filters the children', () {
      final rows = threads();
      resolveRelations(
        rows,
        [
          plan('comments', 'comment', 'many', 'thread',
              where: {'author': 'user:u1'})
        ],
        fetcher,
      );
      expect((rows.single['comments'] as List).map((c) => c['id']),
          containsAll(['comment:c1', 'comment:c3']));
      expect((rows.single['comments'] as List), hasLength(2));
    });

    test('nested relations resolve a second level', () {
      final rows = threads();
      resolveRelations(
        rows,
        [
          plan('comments', 'comment', 'many', 'thread',
              orderBy: [('score', 'asc')],
              relations: [plan('author', 'user', 'one', 'author')])
        ],
        fetcher,
      );
      final comments = rows.single['comments'] as List;
      expect(((comments.first as Map)['author'] as Map)['name'], 'Linus');
    });

    test('the alias lands last in key order', () {
      final rows = threads();
      resolveRelations(rows, [plan('author', 'user', 'one', 'author')], fetcher);
      expect(rows.single.keys.last, 'author');
    });

    test('nesting past the depth cap throws RelationCycleError', () {
      // Self-join on `id`, so every level keeps matching and the recursion can
      // actually reach the cap (a chain that runs out of children just stops).
      RelationPlan chain(int depth) => plan(
            'self',
            'thread',
            'one',
            'id',
            relations: depth == 0 ? const [] : [chain(depth - 1)],
          );
      expect(
        () => resolveRelations(threads(), [chain(maxRelationDepth)], fetcher),
        throwsA(isA<RelationCycleError>()),
      );
      // One level under the cap is fine.
      expect(
        () => resolveRelations(threads(), [chain(maxRelationDepth - 2)], fetcher),
        returnsNormally,
      );
    });

    test('an empty parent set or plan is a no-op', () {
      resolveRelations([], [plan('author', 'user', 'one', 'author')], fetcher);
      final rows = threads();
      resolveRelations(rows, const [], fetcher);
      // `author` is a real column, so it stays the raw foreign key rather than
      // being replaced by a resolved row.
      expect(rows.single['author'], 'user:u1');
    });
  });

  group('subquery child sync', () {
    late LocalDatabaseService local;
    late StreamProcessorService sp;
    late DataModule data;
    late _ChildRemote remote;
    late Sp00kySync sync;

    setUp(() async {
      local = LocalDatabaseService.open(logger)..provision();
      sp = StreamProcessorService(MemoryPersistenceClient(), logger);
      await sp.init();
      sp.seedPermissionsFromSchema(schemaSurql);
      late DataModule d;
      final cache = CacheModule(local, sp, (u) => d.onStreamUpdate(u), logger);
      d = DataModule(cache, local, schema, logger);
      data = d;
      await data.init('sess');
      remote = _ChildRemote();
      sync = Sp00kySync(
        local,
        RemoteDatabaseService(
            const DatabaseConfig(namespace: 't', database: 't'), remote, logger),
        cache,
        data,
        schema,
        logger,
      );
    });
    tearDown(() async {
      await sync.close();
      data.dispose();
      await sp.close();
      local.close();
    });

    Future<String> registerRelated() => QueryBuilder(
          'thread',
          schema: schema,
          logger: logger,
          registrar: (sql, vars, ttl, relations) =>
              data.query('thread', sql, vars, ttl, relations: relations),
        ).related('comments').run();

    test('registration fetches the child bodies behind the subquery', () async {
      remote.records['thread:t1'] = {
        'id': 'thread:t1',
        'title': 'first',
        '_00_rv': 1
      };
      remote.records['comment:c1'] = {
        'id': 'comment:c1',
        'body': 'hi',
        'thread': 'thread:t1',
        '_00_rv': 1,
      };
      remote.primaryListRef = [
        {'out': 'thread:t1', 'version': 1}
      ];
      remote.subqueryListRef = [
        {'out': 'comment:c1', 'version': 1}
      ];

      final hash = await registerRelated();
      await sync.syncQuery(hash); // pull the primary window
      await sync.registerRemoteQueryForTest(hash);
      await _tick();

      expect(local.getById('comment:c1'), isNotNull,
          reason: 'a child body must be cached so the relation can resolve');
      expect(data.getQueryByHash(hash)!.config.subqueryRemoteArray,
          [('comment:c1', 1)]);
      expect(data.getQueryByHash(hash)!.config.remoteArray, [('thread:t1', 1)],
          reason: 'child rows must not leak into the primary window');
    });

    test('an unchanged child set does not refetch', () async {
      remote.subqueryListRef = [
        {'out': 'comment:c1', 'version': 1}
      ];
      remote.records['comment:c1'] = {
        'id': 'comment:c1',
        'body': 'hi',
        'thread': 'thread:t1',
        '_00_rv': 1,
      };
      final hash = await registerRelated();
      await sync.registerRemoteQueryForTest(hash);
      final firstFetches = remote.fetchedIds.length;
      expect(firstFetches, greaterThan(0));

      await sync.registerRemoteQueryForTest(hash);
      expect(remote.fetchedIds.length, firstFetches,
          reason: 'the child diff is idempotent');
    });

    test('a query with no relations skips the child select entirely', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await sync.registerRemoteQueryForTest(hash);
      expect(remote.queries.any((q) => q.contains('parent IS NOT NONE')), isFalse);
    });
  });

  group('materialization', () {
    late LocalDatabaseService local;
    late StreamProcessorService sp;
    late DataModule data;

    setUp(() async {
      local = LocalDatabaseService.open(logger)..provision();
      sp = StreamProcessorService(MemoryPersistenceClient(), logger);
      await sp.init();
      sp.seedPermissionsFromSchema(schemaSurql);
      late DataModule d;
      final cache = CacheModule(local, sp, (u) => d.onStreamUpdate(u), logger);
      d = DataModule(cache, local, schema, logger, bornFetching: false);
      data = d;
      await data.init('sess');
    });
    tearDown(() async {
      data.dispose();
      await sp.close();
      local.close();
    });

    test('a registered query attaches its relations to every row', () async {
      local.create('user:u1', {'name': 'Ada'});
      local.create('thread:t1', {'title': 'first', 'author': 'user:u1'});
      local.create('comment:c1',
          {'body': 'hi', 'thread': 'thread:t1', 'score': 1, 'author': 'user:u1'});

      final builder = QueryBuilder(
        'thread',
        schema: schema,
        logger: logger,
        registrar: (sql, vars, ttl, relations) =>
            data.query('thread', sql, vars, ttl, relations: relations),
      )
          .related('author')
          .related('comments');
      final hash = await builder.run();

      // Ingest so the circuit's view (and therefore the result set) is non-empty.
      await data.onStreamUpdate(StreamUpdate(
        queryHash: hash,
        op: 'CREATE',
        localArray: [('thread:t1', 1)],
      ));

      final row = data.getQueryByHash(hash)!.records.single;
      expect((row['author'] as Map)['name'], 'Ada');
      expect((row['comments'] as List).single['id'], 'comment:c1');
    });

    test('materializing does not mutate the cached document', () async {
      local.create('user:u1', {'name': 'Ada'});
      local.create('thread:t1', {'title': 'first', 'author': 'user:u1'});

      final hash = await QueryBuilder(
        'thread',
        schema: schema,
        logger: logger,
        registrar: (sql, vars, ttl, relations) =>
            data.query('thread', sql, vars, ttl, relations: relations),
      ).related('comments').run();

      await data.onStreamUpdate(StreamUpdate(
        queryHash: hash,
        op: 'CREATE',
        localArray: [('thread:t1', 1)],
      ));

      expect(data.getQueryByHash(hash)!.records.single['comments'], isEmpty);
      expect(local.getById('thread:t1')!.containsKey('comments'), isFalse,
          reason: 'the alias must not be written back into the store');
    });
  });
}
