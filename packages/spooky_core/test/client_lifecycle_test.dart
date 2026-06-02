import 'package:spooky_core/spooky_core.dart';
import 'package:test/test.dart';

/// Local-only Sp00kyClient surface: init, queries as Streams, callback
/// subscribe, mutation counts, CRDT stubs, and double-init/close safety.
void main() {
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };
  const schemaSurql = 'DEFINE TABLE thread PERMISSIONS FOR select WHERE true;';

  late Sp00kyClient client;
  setUp(() async {
    client = Sp00kyClient(Sp00kyConfig(
      database: const DatabaseConfig(namespace: 't', database: 't'),
      schema: schema,
      schemaSurql: schemaSurql,
    ));
    await client.init();
  });
  tearDown(() => client.close());

  test('init is idempotent', () async {
    await client.init(); // second call is a no-op
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    expect(hash, isNotEmpty);
  });

  test('queryStream registers and emits; create propagates', () async {
    final stream = await client.queryStream('SELECT * FROM thread', {});
    final emissions = <List<Map<String, dynamic>>>[];
    final sub = stream.listen(emissions.add);
    await client.create('thread:a', {'title': 'hi'});
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(emissions.last.map((r) => r['id']), contains('thread:a'));
    await sub.cancel();
  });

  test('callback subscribe receives the current set immediately', () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    await client.create('thread:a', {'title': 'hi'});
    await Future<void>.delayed(const Duration(milliseconds: 30));

    List<Map<String, dynamic>>? latest;
    final off = client.subscribe(hash, (r) => latest = r, immediate: true);
    expect(latest, isNotNull);
    expect(latest!.any((r) => r['id'] == 'thread:a'), isTrue);
    off();
  });

  test('multiple Stream listeners on one hash both receive updates', () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    final a = <List<Map<String, dynamic>>>[];
    final b = <List<Map<String, dynamic>>>[];
    final s1 = client.subscribeStream(hash).listen(a.add);
    final s2 = client.subscribeStream(hash).listen(b.add);
    await client.create('thread:a', {'title': 'hi'});
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(a.last.any((r) => r['id'] == 'thread:a'), isTrue);
    expect(b.last.any((r) => r['id'] == 'thread:a'), isTrue);
    await s1.cancel();
    await s2.cancel();
  });

  test('pendingMutationCount is 0 without a remote sync', () {
    expect(client.pendingMutationCount, 0);
    expect(client.liveRetryCount, 0);
  });

  test('CRDT entry points are stubbed (deferred)', () {
    expect(() => client.openCrdtField('thread', 'thread:a', 'content'),
        throwsA(isA<UnimplementedError>()));
    expect(() => client.closeCrdtField('thread', 'thread:a', 'content'),
        throwsA(isA<UnimplementedError>()));
  });

  test('auth getter throws without a remote endpoint', () {
    expect(() => client.auth, throwsA(isA<StateError>()));
  });

  test('queryRaw rejects an unparseable query', () {
    expect(
        () => client.queryRaw('RETURN 1', {}), throwsA(isA<ArgumentError>()));
  });
}
