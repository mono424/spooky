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
    if (sql.contains(r'$auth.id')) {
      return [
        [
          {'id': 'user:u1'}
        ]
      ];
    }
    if (sql.contains('session::id()')) return ['sess-1'];
    if (sql.contains('fn::query::register')) return [null];
    if (sql.contains('FROM _00_list_ref')) {
      // Initial fetch: empty list_ref.
      return [<dynamic>[]];
    }
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
