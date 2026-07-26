import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// `applyHydration` ingests one-shot rows so a cold query displays immediately,
/// and `persistSnapshot` does the same without registering a query at all.
void main() {
  final logger = SpookyLogger.root('test');
  const schemaSurql = 'DEFINE TABLE thread PERMISSIONS FOR select WHERE true;'
      'DEFINE TABLE comment PERMISSIONS FOR select WHERE true;';
  final schema = {
    'thread': {
      'columns': {
        'title': const ColumnSchema(type: 'string'),
        'author': const ColumnSchema(type: 'record', recordId: true),
      },
    },
    'comment': {
      'columns': {
        'body': const ColumnSchema(type: 'string'),
        'thread': const ColumnSchema(type: 'record', recordId: true),
        'author': const ColumnSchema(type: 'record', recordId: true),
      },
    },
    'user': {
      'columns': {'name': const ColumnSchema(type: 'string')},
    },
  };

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
    d = DataModule(cache, local, schema, logger);
    data = d;
    await data.init('sess');
  });
  tearDown(() async {
    data.dispose();
    await sp.close();
    local.close();
  });

  group('isCold', () {
    test('a fresh query is cold, a fetched one is not', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      expect(data.isCold(hash), isTrue);
      await data.updateQueryRemoteArray(hash, [('thread:a', 1)]);
      expect(data.isCold(hash), isFalse,
          reason: 'a query that already fetched its window is warm');
    });

    test('an unknown query is not cold', () {
      expect(data.isCold('nope'), isFalse);
    });

    test('hydrating marks the query warm even with no rows', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, []);
      expect(data.isCold(hash), isFalse, reason: 'run-once, even when empty');
    });
  });

  group('applyHydration', () {
    test('persists rows, primes remoteArray, and notifies subscribers',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final emissions = <List<Map<String, dynamic>>>[];
      data.subscribe(hash, emissions.add);

      await data.applyHydration(hash, [
        {'id': 'thread:a', 'title': 'hi', '_00_rv': 3},
      ]);

      expect(local.getById('thread:a')!['title'], 'hi');
      expect(data.getQueryByHash(hash)!.config.remoteArray, [('thread:a', 3)]);
      // Both the hydration notify and the resulting circuit ingest emit; what
      // matters is that subscribers see the hydrated row.
      expect(emissions, isNotEmpty);
      expect(emissions.last.map((r) => r['id']), ['thread:a']);
    });

    test('defaults a missing _00_rv to version 1', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {'id': 'thread:a', 'title': 'hi'},
      ]);
      expect(data.getQueryByHash(hash)!.config.remoteArray, [('thread:a', 1)]);
    });

    test('is a no-op for an unknown query', () async {
      await data.applyHydration('nope', [
        {'id': 'thread:a', 'title': 'hi'},
      ]);
      expect(local.getById('thread:a'), isNull);
    });

    test('closes the cold window so a caller never hydrates twice', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {'id': 'thread:a', 'title': 'first', '_00_rv': 1},
      ]);
      // The run-once guard is the caller's isCold check (the client's background
      // init chain); hydrating flips both the flag and remoteArray, so a second
      // pass can't land a staler snapshot on top.
      expect(data.isCold(hash), isFalse);
      expect(data.getQueryByHash(hash)!.hydrated, isTrue);
    });

    test('seeds the correct window for an offset query', () async {
      final hash = await data.query('thread',
          'SELECT * FROM thread ORDER BY title asc LIMIT 2 START 2', {}, '10m');
      await data.applyHydration(hash, [
        {'id': 'thread:d', 'title': 'd', '_00_rv': 1},
        {'id': 'thread:c', 'title': 'c', '_00_rv': 1},
      ]);
      expect(data.getQueryByHash(hash)!.records.map((r) => r['id']),
          ['thread:c', 'thread:d']);
    });
  });

  group('embedded relation extraction', () {
    test('a forward relation object is stored as its id, child cached too',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {
          'id': 'thread:a',
          'title': 'hi',
          'author': {'id': 'user:u1', 'name': 'Ada', '_00_rv': 1},
        },
      ]);

      expect(local.getById('thread:a')!['author'], 'user:u1',
          reason: 'the parent keeps a reference, not the nested body');
      expect(local.getById('user:u1')!['name'], 'Ada',
          reason: 'the child is cached as its own row');
    });

    test('a reverse subquery array is dropped, children cached separately',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {
          'id': 'thread:a',
          'title': 'hi',
          'comments': [
            {'id': 'comment:c1', 'body': 'one', '_00_rv': 1},
            {'id': 'comment:c2', 'body': 'two', '_00_rv': 1},
          ],
        },
      ]);

      expect(local.getById('thread:a')!.containsKey('comments'), isFalse);
      expect(local.getById('comment:c1')!['body'], 'one');
      expect(local.getById('comment:c2')!['body'], 'two');
    });

    test('nested grandchildren are captured', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {
          'id': 'thread:a',
          'title': 'hi',
          'comments': [
            {
              'id': 'comment:c1',
              'body': 'one',
              'author': {'id': 'user:u1', 'name': 'Ada'},
            },
          ],
        },
      ]);
      expect(local.getById('user:u1')!['name'], 'Ada');
      expect(local.getById('comment:c1')!['author'], 'user:u1');
    });

    test('a bare id reference is not mistaken for an embedded body', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.applyHydration(hash, [
        {'id': 'thread:a', 'title': 'hi', 'author': 'user:u1'},
      ]);
      expect(local.getById('thread:a')!['author'], 'user:u1');
      expect(local.getById('user:u1'), isNull,
          reason: 'a foreign key carries no body to cache');
    });
  });

  group('persistSnapshot', () {
    test('caches rows without registering a query', () async {
      await data.persistSnapshot('thread', [
        {'id': 'thread:a', 'title': 'warm', '_00_rv': 1},
      ]);
      expect(local.getById('thread:a')!['title'], 'warm');
      expect(data.getActiveQueryHashes(), isEmpty,
          reason: 'prewarming must not create a live view');
    });

    test('an empty snapshot is a no-op', () async {
      await data.persistSnapshot('thread', []);
      expect(local.getAll('thread'), isEmpty);
    });
  });

  group('preload markers', () {
    test('absent until written, then round-trips', () {
      expect(data.getPreloadMarker('h1'), isNull);
      data.writePreloadMarker('h1', 7);
      final marker = data.getPreloadMarker('h1');
      expect(marker, isNotNull);
      expect(marker!.rowCount, 7);
      expect(marker.fetchedAt, greaterThan(0));
    });

    test('a malformed marker row reads as cold', () {
      local.replace('_00_preload:h2', {'garbage': true});
      final marker = data.getPreloadMarker('h2');
      expect(marker!.fetchedAt, 0, reason: 'unusable marker reads as stale');
      expect(marker.rowCount, 0);
    });

    test('markers do not leak into query results', () async {
      data.writePreloadMarker('h1', 1);
      await data.persistSnapshot('thread', [
        {'id': 'thread:a', 'title': 'x', '_00_rv': 1},
      ]);
      expect(local.getAll('thread').map((r) => r['id']), ['thread:a']);
    });
  });
}
