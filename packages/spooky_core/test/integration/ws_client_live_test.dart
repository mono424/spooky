@Tags(['integration'])
library;

import 'dart:async';
import 'dart:io';

import 'package:spooky_core/src/surreal/remote_client.dart';
import 'package:spooky_core/src/surreal/value.dart';
import 'package:test/test.dart';

/// Live integration test for [WebSocketSurrealClient] against a real SurrealDB.
///
/// Boot one first, e.g.:
///   docker run -d --name spooky-surreal-it -p 18011:8000 \
///     surrealdb/surrealdb:v2.1.4 start --user root --pass root --allow-all memory
///
/// Override the endpoint with SURREAL_IT_ENDPOINT. Skips if unreachable.
void main() {
  final endpoint =
      Platform.environment['SURREAL_IT_ENDPOINT'] ?? 'ws://127.0.0.1:18011';

  late WebSocketSurrealClient client;

  setUp(() async {
    client = WebSocketSurrealClient();
    try {
      await client.connect(endpoint).timeout(const Duration(seconds: 3));
      // Root signin so we can run DDL/DML.
      await client.signin({'user': 'root', 'pass': 'root'});
      await client.use(namespace: 'it', database: 'it');
    } catch (e) {
      await client.close();
      markTestSkipped('SurrealDB not reachable at $endpoint: $e');
      return;
    }
  });

  tearDown(() async {
    try {
      await client.query('REMOVE TABLE thing');
    } catch (_) {}
    await client.close();
  });

  test('query: create then select round-trips', () async {
    await client.query(r'CREATE thing:a SET name = $name', {'name': 'alice'});
    final results = await client.query('SELECT * FROM thing');
    expect(results, isNotEmpty);
    final rows = results.first as List;
    expect(rows, hasLength(1));
    expect((rows.first as Map)['name'], 'alice');
  });

  test('bind vars: RecordId encodes and matches', () async {
    await client.query(r'CREATE $id SET v = 1', {'id': RecordId('thing', 'x')});
    final results = await client
        .query(r'SELECT * FROM $id', {'id': RecordId('thing', 'x')});
    final rows = results.first as List;
    expect(rows, hasLength(1));
    expect((rows.first as Map)['v'], 1);
  });

  test('a per-statement ERR throws', () async {
    await expectLater(
      client.query("THROW 'boom'"), // per-statement ERR on v2 and v3
      throwsA(isA<StateError>()),
    );
  });

  test('LIVE: receives a notification on insert, stops after KILL', () async {
    // v3 rejects LIVE on a non-existent table; seed a row so it exists.
    await client.query('CREATE thing:seed SET name = "seed"');
    final (liveId, stream) = await client.live('LIVE SELECT * FROM thing');
    expect(liveId, isNotEmpty);

    final got = <LiveMessage>[];
    final sub = stream.listen(got.add);

    await client.query(r'CREATE thing:live SET name = $n', {'n': 'bob'});
    await _waitFor(() => got.isNotEmpty, const Duration(seconds: 3));

    expect(got, isNotEmpty);
    expect(got.first.action, anyOf('CREATE', 'UPDATE'));
    expect(got.first.value['name'], 'bob');

    await client.kill(liveId);
    final countAfterKill = got.length;
    await client.query(r'CREATE thing:live2 SET name = $n', {'n': 'carol'});
    await Future<void>.delayed(const Duration(milliseconds: 500));
    expect(got.length, countAfterKill, reason: 'no notifications after KILL');

    await sub.cancel();
  });

  test('connected lifecycle event fires', () async {
    final c = WebSocketSurrealClient();
    final connected = Completer<void>();
    c.onConnected.listen((_) => connected.complete());
    await c.connect(endpoint);
    await connected.future.timeout(const Duration(seconds: 2));
    await c.close();
  });
}

Future<void> _waitFor(bool Function() pred, Duration timeout) async {
  final deadline = DateTime.now().add(timeout);
  while (!pred() && DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 25));
  }
}
