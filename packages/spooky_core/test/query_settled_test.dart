import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:test/test.dart';

import 'sync_integration_test.dart' show FakeRemote;

/// The settled contract: a query holds `fetching` across its WHOLE registration
/// and only flips to `idle` after its rows have landed. A consumer that treats
/// `idle` as "this window is complete" (a virtualized list sizing itself to a
/// short page) depends on that ordering.
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
        // Keep the poll from racing extra fetch cycles into these assertions.
        refSyncIntervalMs: 60000,
      ),
      remoteClient: remote,
    );
    await client.init();
    await _settle();
  });
  tearDown(() => client.close());

  test('a registration settles to idle exactly once, after its rows land',
      () async {
    remote.records['thread:a'] = {
      'id': 'thread:a',
      'title': 'from server',
      '_00_rv': 1,
    };
    remote.listRef = [
      {'out': 'thread:a', 'version': 1}
    ];

    final hash = await client.queryRaw('SELECT * FROM thread', {});
    // Snapshot the records visible at each status change: `idle` must not arrive
    // before the fetched row is materialized.
    final observed = <(QueryStatus, int)>[];
    client.subscribeQueryStatus(
      hash,
      (s) => observed
          .add((s, client.dataModule.getQueryByHash(hash)?.records.length ?? 0)),
      immediate: true,
    );
    await _settle();

    expect(observed.first.$1, QueryStatus.fetching,
        reason: 'a fresh query is born fetching');
    expect(observed.last.$1, QueryStatus.idle);
    expect(observed.where((o) => o.$1 == QueryStatus.idle), hasLength(1),
        reason: 'exactly one settle for one registration');
    expect(observed.last.$2, 1,
        reason: 'the row must be materialized before idle');
  });

  test('an empty result set still settles to idle', () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    final seen = <QueryStatus>[];
    client.subscribeQueryStatus(hash, seen.add, immediate: true);
    await _settle();
    expect(seen, [QueryStatus.fetching, QueryStatus.idle]);
  });

  test('an empty result set notifies subscribers so loading can stop', () async {
    final hash = await client.queryRaw('SELECT * FROM thread', {});
    final emissions = <List<Map<String, dynamic>>>[];
    client.subscribe(hash, emissions.add); // no immediate replay
    await _settle();
    expect(emissions, isNotEmpty,
        reason: 'notifyQuerySynced must fire even with no rows');
    expect(emissions.last, isEmpty);
  });

}

Future<void> _settle() =>
    Future<void>.delayed(const Duration(milliseconds: 80));
