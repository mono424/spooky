import 'dart:async';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:test/test.dart';

/// Drives the full sync orchestration (up-queue -> remote, down-queue register
/// + initial fetch, LIVE -> down-sync) against a fake remote client, with no
/// real SurrealDB server.
void main() {
  late FakeRemote remote;
  late Sp00kyClient client;

  const schemaSurql =
      'DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;';
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };

  setUp(() async {
    remote = FakeRemote();
    // Pre-seed a token so AuthService.check() hydrates a user and starts LIVE.
    final persistence = MemoryPersistenceClient();
    await persistence.set('sp00ky_auth_token', 'tok');
    client = Sp00kyClient(
      Sp00kyConfig(
        database: const DatabaseConfig(
          endpoint: 'ws://localhost:8000',
          namespace: 'test',
          database: 'test',
        ),
        schema: schema,
        schemaSurql: schemaSurql,
        persistenceClient: persistence,
      ),
      remoteClient: remote,
    );
    await client.init();
    await _settle(); // let the auth-driven LIVE subscription establish
  });

  tearDown(() => client.close());

  test('init connects, uses ns/db, and fetches session id', () {
    expect(remote.connected, isTrue);
    expect(remote.usedNamespace, 'test');
    expect(remote.queries.any((q) => q.contains('session::id()')), isTrue);
  });

  test('local create is pushed to remote as a CREATE up-event', () async {
    await client.create('thread:a', {'title': 'hello'});
    await _settle();
    expect(
      remote.queries.any((q) => q.contains('CREATE ONLY \$id SET')),
      isTrue,
      reason: 'expected a create statement sent to remote',
    );
  });

  test('queryRaw registers the query remotely and fetches list_ref', () async {
    await client.queryRaw('SELECT * FROM thread', {});
    await _settle();
    expect(
        remote.queries.any((q) => q.contains('fn::query::register')), isTrue);
    expect(remote.queries.any((q) => q.contains('FROM _00_list_ref')), isTrue);
  });

  test('a remote LIVE create flows into the query Stream', () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    await _settle();

    final emissions = <List<Map<String, dynamic>>>[];
    final sub = client.subscribeStream(hash).listen(emissions.add);
    await _settle();

    // Seed the record the engine will fetch, then push a LIVE list_ref CREATE.
    final queryId = 'query:$hash';
    remote.records['thread:remote'] = {
      'id': 'thread:remote',
      'title': 'live',
      '_00_rv': 1
    };
    remote.pushLive('CREATE', {
      'in': queryId,
      'out': 'thread:remote',
      'version': 1,
    });
    await _settle();

    expect(
      emissions.expand((e) => e).any((r) => r['id'] == 'thread:remote'),
      isTrue,
      reason: 'LIVE-delivered record should reach the Stream',
    );
    await sub.cancel();
  });

  // Reproduces the "records sync one by one" report. The batch-coalescing
  // window only collapses a SINGLE saveBatch (the initial registration fetch).
  // The LIVE path is different: SurrealDB delivers one `_00_list_ref` message
  // per record, and `_handleRemoteListRefChange` turns each into a one-record
  // diff -> its own `syncRecords` -> its own `saveBatch([one])`. So a burst of
  // N records that land together still produces N separate stream emissions,
  // each growing the list by one -> the UI renders row-by-row.
  test('a burst of remote LIVE creates syncs one record at a time (bug repro)',
      () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    await _settle();

    // Seed three records the engine will fetch when their list_ref CREATE lands.
    final queryId = 'query:$hash';
    for (var i = 1; i <= 3; i++) {
      remote.records['thread:r$i'] = {
        'id': 'thread:r$i',
        'title': 'live $i',
        '_00_rv': 1,
      };
    }

    final emissions = <List<Map<String, dynamic>>>[];
    // immediate: false so we only capture emissions caused by the LIVE burst,
    // not the initial (empty) snapshot on subscribe.
    final sub = client
        .subscribeStream(hash, immediate: false)
        .listen((e) => emissions.add(e));
    await _settle();

    // A burst: three list_ref CREATEs pushed back-to-back, as a remote bulk
    // insert of three rows into the same query would deliver them.
    for (var i = 1; i <= 3; i++) {
      remote.pushLive('CREATE', {
        'in': queryId,
        'out': 'thread:r$i',
        'version': 1,
      });
    }
    await _settle();

    // All three records arrive...
    expect(emissions.last.map((r) => r['id']),
        containsAll(['thread:r1', 'thread:r2', 'thread:r3']));

    // ...but one emission per record (the list grows 1 -> 2 -> 3), instead of a
    // single coalesced emission. This is the row-by-row sync being reproduced.
    expect(
      emissions.length,
      3,
      reason: 'LIVE burst currently emits once per record (row-by-row sync)',
    );
    expect(
      emissions.map((e) => e.length).toList(),
      [1, 2, 3],
      reason: 'each emission adds exactly one more record',
    );

    await sub.cancel();
  });
}

Future<void> _settle() =>
    Future<void>.delayed(const Duration(milliseconds: 60));

/// In-memory fake of [RemoteSurrealClient]. Records queries, answers the few
/// shapes the sync layer issues, and lets tests push LIVE messages.
class FakeRemote implements RemoteSurrealClient {
  bool connected = false;
  String? usedNamespace;
  final List<String> queries = [];
  final Map<String, Map<String, dynamic>> records = {};

  /// Rows the `_00_list_ref` select returns (i.e. the server's view membership).
  /// Empty by default so a registration finds no records.
  List<Map<String, dynamic>> listRef = [];

  /// Make mutation pushes hang, so an up-event stays pending in the outbox.
  bool blockMutations = false;

  final _connected = StreamController<void>.broadcast();
  final _disconnected = StreamController<void>.broadcast();
  final _live = StreamController<LiveMessage>.broadcast();

  @override
  Stream<void> get onConnected => _connected.stream;
  @override
  Stream<void> get onDisconnected => _disconnected.stream;

  void pushLive(String action, Map<String, dynamic> value) =>
      _live.add(LiveMessage(action, value));

  @override
  Future<void> connect(String endpoint) async => connected = true;

  @override
  Future<void> use(
      {required String namespace, required String database}) async {
    usedNamespace = namespace;
  }

  @override
  Future<dynamic> authenticate(String token) async => null;
  @override
  Future<dynamic> signin(Map<String, dynamic> params) async =>
      {'access': 'tok'};
  @override
  Future<dynamic> signup(Map<String, dynamic> params) async =>
      {'access': 'tok'};
  @override
  Future<void> invalidate() async {}

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    queries.add(sql);
    if (blockMutations &&
        (sql.contains('CREATE ONLY') ||
            sql.startsWith('UPDATE') ||
            sql.startsWith('DELETE'))) {
      // A network-classified failure keeps the event queued in the outbox
      // instead of rolling it back, so it stays pending.
      throw Exception('connection refused');
    }
    if (sql.contains(r'$auth.id')) {
      return [
        [
          {'id': 'user:u1'}
        ]
      ];
    }
    if (sql.contains('session::id()')) return ['sess-1'];
    if (sql.contains('fn::query::register')) return [null];
    if (sql.contains('FROM _00_list_ref')) return [listRef];
    if (sql.contains('FROM \$idsToFetch')) {
      final ids = (vars?['idsToFetch'] as List?) ?? const [];
      return [
        ids
            .map((id) => records[id.toString()])
            .where((r) => r != null)
            .toList(),
      ];
    }
    if (sql.contains('type::table')) return [<dynamic>[]];
    return [null];
  }

  @override
  Future<(String, Stream<LiveMessage>)> live(String sql,
      [Map<String, dynamic>? vars]) async {
    queries.add(sql);
    return ('live-1', _live.stream);
  }

  @override
  Future<void> kill(String liveId) async {}

  @override
  Future<void> close() async {
    await _connected.close();
    await _disconnected.close();
    await _live.close();
  }
}
