import 'dart:async';
import 'dart:io';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:test/test.dart';

import 'sync_integration_test.dart' show FakeRemote;

/// `preload` prewarms the local cache without registering a live view, and the
/// background init chain hydrates a cold query strictly BEFORE enqueuing its
/// `register` (so a one-shot snapshot can never overwrite the authoritative
/// list_ref result).
void main() {
  const schemaSurql =
      'DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;';
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };

  late _PreloadRemote remote;
  late Sp00kyClient client;
  late String dbPath;

  /// A client over a FILE-backed store, so the durable `_00_preload` marker
  /// survives the simulated app restarts the warm-path tests need.
  Future<Sp00kyClient> makeClient({bool instantHydrate = false}) async {
    final persistence = MemoryPersistenceClient();
    await persistence.set('sp00ky_auth_token', 'tok');
    final c = Sp00kyClient(
      Sp00kyConfig(
        database: DatabaseConfig(
          endpoint: 'ws://localhost:8000',
          namespace: 'test',
          database: 'test',
          store: StoreType.indexeddb,
          localDbPath: dbPath,
        ),
        schema: schema,
        schemaSurql: schemaSurql,
        persistenceClient: persistence,
        instantHydrate: instantHydrate,
        refSyncIntervalMs: 60000,
      ),
      remoteClient: remote,
    );
    await c.init();
    await _settle();
    return c;
  }

  setUp(() async {
    final dir = Directory.systemTemp.createTempSync('spooky_preload');
    addTearDown(() => dir.deleteSync(recursive: true));
    dbPath = '${dir.path}/spooky.db';
    remote = _PreloadRemote();
    client = await makeClient();
  });
  tearDown(() => client.close());

  group('preload', () {
    test('a cold preload fetches, persists, and stamps the marker', () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ];

      await client.preload('SELECT * FROM thread', {});

      expect(client.local.getById('thread:a')!['title'], 'warm');
      expect(remote.selectCount, 1);
      expect(client.dataModule.getActiveQueryHashes(), isEmpty,
          reason: 'prewarming must not register a live view');
    });

    test('a second preload in the same session is free', () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});
      await client.preload('SELECT * FROM thread', {});
      expect(remote.selectCount, 1, reason: 'deduped by query hash');
    });

    test('a failed fetch stamps no marker, so it retries next time', () async {
      remote.selectError = Exception('connection refused');
      await client.preload('SELECT * FROM thread', {});
      expect(remote.selectCount, 1);

      remote.selectError = null;
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});
      expect(remote.selectCount, 2, reason: 'no marker was written');
      expect(client.local.getById('thread:a'), isNotNull);
    });

    test('warm + onUse (default) never refetches', () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});
      await client.close();

      // A new session over the same store: the durable marker makes it warm.
      client = await makeClient();
      await client.preload('SELECT * FROM thread', {});
      await _settle();
      expect(remote.selectCount, 1);
    });

    test('warm + background kicks a silent refetch', () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'v1', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});
      await client.close();

      client = await makeClient();
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'v2', '_00_rv': 2},
      ];
      await client.preload('SELECT * FROM thread', {},
          options: const PreloadOptions(refresh: PreloadRefresh.background));
      await _settle();
      expect(remote.selectCount, 2);
      expect(client.local.getById('thread:a')!['title'], 'v2');
    });

    test('warm + stale respects staleTime', () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'v1', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});
      await client.close();

      client = await makeClient();
      // The marker was just written, so a 1h staleTime leaves it fresh.
      await client.preload('SELECT * FROM thread', {},
          options: const PreloadOptions(refresh: PreloadRefresh.stale));
      await _settle();
      expect(remote.selectCount, 1);

      // A zero staleTime makes anything stale.
      await client.close();
      client = await makeClient();
      await client.preload('SELECT * FROM thread', {},
          options: const PreloadOptions(
              refresh: PreloadRefresh.stale, staleTime: '0s'));
      await _settle();
      expect(remote.selectCount, 2);
    });

    test('a preloaded query paints from cache on its first registration',
        () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ];
      await client.preload('SELECT * FROM thread', {});

      // Registration returns as soon as the LOCAL registration completes, and the
      // prewarmed row is already materialized.
      final hash = await client.queryRaw('SELECT * FROM thread', {});
      expect(client.dataModule.getQueryByHash(hash)!.records.map((r) => r['id']),
          ['thread:a']);
    });

    test('throws without a remote endpoint', () async {
      final localOnly = Sp00kyClient(Sp00kyConfig(
        database: const DatabaseConfig(namespace: 't', database: 't'),
        schema: schema,
        schemaSurql: schemaSurql,
      ));
      await localOnly.init();
      expect(() => localOnly.preload('SELECT * FROM thread', {}),
          throwsA(isA<StateError>()));
      await localOnly.close();
    });
  });

  group('background init chain', () {
    test('instant-hydrate off: no one-shot fetch, only the registration',
        () async {
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'hi', '_00_rv': 1},
      ];
      await client.queryRaw('SELECT * FROM thread', {});
      await _settle();
      expect(remote.selectCount, 0, reason: 'hydrate is opt-in');
      expect(remote.queries.any((q) => q.contains('fn::query::register')),
          isTrue);
    });

    test('instant-hydrate on: hydrates a cold query before registering it',
        () async {
      await client.close();
      client = await makeClient(instantHydrate: true);
      // Keep the fake self-consistent: the row the one-shot returns is also in
      // the query's server-side window, else the following registration would
      // (correctly) evict it as a record that left the view.
      remote.selectRows = [
        {'id': 'thread:a', 'title': 'hi', '_00_rv': 1},
      ];
      remote.listRef = [
        {'out': 'thread:a', 'version': 1}
      ];
      remote.records['thread:a'] = {
        'id': 'thread:a',
        'title': 'hi',
        '_00_rv': 1
      };

      final hash = await client.queryRaw('SELECT * FROM thread', {});
      await _settle();

      expect(remote.selectCount, 1);
      expect(client.local.getById('thread:a'), isNotNull);
      // Ordering is load-bearing: a hydrate landing after the register's
      // list_ref fetch would overwrite the authoritative result with a snapshot.
      final hydrateAt =
          remote.queries.indexWhere((q) => q == 'SELECT * FROM thread');
      final registerAt =
          remote.queries.indexWhere((q) => q.contains('fn::query::register'));
      expect(hydrateAt, greaterThanOrEqualTo(0));
      expect(registerAt, greaterThan(hydrateAt));
      expect(client.dataModule.isCold(hash), isFalse);
    });

    test('instant-hydrate skips a warm query', () async {
      await client.close();
      client = await makeClient(instantHydrate: true);
      remote.listRef = [
        {'out': 'thread:a', 'version': 1}
      ];
      remote.records['thread:a'] = {
        'id': 'thread:a',
        'title': 'hi',
        '_00_rv': 1
      };

      final hash = await client.queryRaw('SELECT * FROM thread', {});
      await _settle();
      final firstFetches = remote.selectCount;

      // Re-register the same query: it already fetched its window, so no hydrate.
      await client.queryRaw('SELECT * FROM thread', {});
      await _settle();
      expect(remote.selectCount, firstFetches);
      expect(client.dataModule.isCold(hash), isFalse);
    });

    test('a hydrate failure still registers the query', () async {
      await client.close();
      client = await makeClient(instantHydrate: true);
      remote.selectError = Exception('connection refused');

      await client.queryRaw('SELECT * FROM thread', {});
      await _settle();
      expect(remote.queries.any((q) => q.contains('fn::query::register')),
          isTrue);
    });

    test('concurrent registrations of one query share a single chain', () async {
      await Future.wait([
        client.queryRaw('SELECT * FROM thread', {}),
        client.queryRaw('SELECT * FROM thread', {}),
        client.queryRaw('SELECT * FROM thread', {}),
      ]);
      await _settle();
      expect(
        remote.queries.where((q) => q.contains('fn::query::register')).length,
        1,
      );
    });
  });
}

Future<void> _settle() =>
    Future<void>.delayed(const Duration(milliseconds: 80));

/// [FakeRemote] plus a scriptable one-shot `SELECT * FROM thread` (the shape the
/// hydrate/preload path issues), so the test can count and fail those fetches.
class _PreloadRemote extends FakeRemote {
  List<Map<String, dynamic>> selectRows = [];
  Object? selectError;
  int selectCount = 0;

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    // The plain table select is only issued by preload / instant-hydrate; the
    // sync paths use `fn::query::register`, `_00_list_ref` and `$idsToFetch`.
    if (sql.startsWith('SELECT * FROM thread')) {
      queries.add(sql);
      selectCount++;
      if (selectError != null) throw selectError!;
      return [selectRows];
    }
    return super.query(sql, vars);
  }
}
