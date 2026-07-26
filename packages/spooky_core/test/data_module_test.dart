import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

void main() {
  final logger = SpookyLogger.root('test');
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };

  late LocalDatabaseService local;
  late StreamProcessorService sp;
  late DataModule data;

  setUp(() async {
    local = LocalDatabaseService.open(logger)..provision();
    sp = StreamProcessorService(MemoryPersistenceClient(), logger);
    await sp.init();
    sp.seedPermissionsFromSchema(
        'DEFINE TABLE thread PERMISSIONS FOR select WHERE true;');
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

  test('concurrent query() for the same surql dedupes to one registration',
      () async {
    final results = await Future.wait([
      data.query('thread', 'SELECT * FROM thread', {}, '10m'),
      data.query('thread', 'SELECT * FROM thread', {}, '10m'),
    ]);
    expect(results[0], results[1]); // same hash
    expect(data.getActiveQueryHashes(), hasLength(1));
  });

  test('calculateHash includes the session salt', () {
    final h1 =
        data.calculateHash({'surql': 'SELECT * FROM thread', 'params': {}});
    data.setSessionId('other');
    final h2 =
        data.calculateHash({'surql': 'SELECT * FROM thread', 'params': {}});
    expect(h1, isNot(h2));
  });

  test('create emits a CreateEvent to mutation subscribers', () async {
    final events = <UpEvent>[];
    data.onMutation((m) => events.addAll(m));
    await data.create('thread:a', {'title': 'hi'});
    expect(events.single, isA<CreateEvent>());
    expect(events.single.recordId.encode(), 'thread:a');
  });

  test('update emits an UpdateEvent carrying the before-record', () async {
    await data.create('thread:a', {'title': 'v1'});
    final events = <UpEvent>[];
    data.onMutation((m) => events.addAll(m));
    await data.update('thread', 'thread:a', {'title': 'v2'});
    final ev = events.single as UpdateEvent;
    expect(ev.beforeRecord!['title'], 'v1');
    expect(local.getById('thread:a')!['title'], 'v2');
  });

  test('rollbackCreate deletes the record and notifies subscribers', () async {
    final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
    await data.create('thread:a', {'title': 'hi'});

    final emissions = <List<Map<String, dynamic>>>[];
    data.subscribe(hash, emissions.add, immediate: true);
    await Future<void>.delayed(const Duration(milliseconds: 20));

    await data.rollbackCreate(RecordId.parse('thread:a'), 'thread');
    expect(local.getById('thread:a'), isNull);
    expect(emissions.last.any((r) => r['id'] == 'thread:a'), isFalse);
  });

  test('rollbackUpdate restores the previous record', () async {
    await data.create('thread:a', {'title': 'v1'});
    await data.update('thread', 'thread:a', {'title': 'v2'});
    await data.rollbackUpdate(RecordId.parse('thread:a'), 'thread',
        {'id': 'thread:a', 'title': 'v1', '_00_rv': 1});
    expect(local.getById('thread:a')!['title'], 'v1');
  });

  test('updateQueryRemoteArray persists to the _00_query config', () async {
    final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
    await data.updateQueryRemoteArray(hash, [('thread:a', 2)]);
    final cfg = local.getQueryConfig('_00_query:$hash')!;
    expect(cfg['remoteArray'], [
      ['thread:a', 2]
    ]);
  });

  group('windowed materialization', () {
    // The local store is shared and, with sparse windowing, holds only some
    // pages. A page-2 query must take its rows from the server's authoritative
    // list_ref rather than from whatever the local circuit happens to hold —
    // otherwise page 2 renders empty or shows page 1's rows.
    const pageTwo =
        'SELECT * FROM thread ORDER BY title asc LIMIT 2 START 2';

    Future<String> registerPageTwo() =>
        data.query('thread', pageTwo, {}, '10m');

    void seed(String id, String title) => local.create(id, {'title': title});

    test('materializes the remoteArray window when the circuit is empty',
        () async {
      seed('thread:c', 'c');
      seed('thread:d', 'd');
      final hash = await registerPageTwo();
      // The circuit has no rows for this window (nothing was ingested), so the
      // old localArray-only path returned nothing.
      expect(data.getQueryByHash(hash)!.records, isEmpty);

      await data.updateQueryRemoteArray(hash, [('thread:d', 1), ('thread:c', 1)]);
      await data.notifyQuerySynced(hash);

      final records = data.getQueryByHash(hash)!.records;
      expect(records.map((r) => r['id']), ['thread:c', 'thread:d'],
          reason: 'window rows come from list_ref, ordered by the query ORDER BY');
    });

    test('re-applies a descending ORDER BY', () async {
      seed('thread:c', 'c');
      seed('thread:d', 'd');
      final hash = await data.query(
          'thread', 'SELECT * FROM thread ORDER BY title desc LIMIT 2 START 2', {}, '10m');
      await data.updateQueryRemoteArray(hash, [('thread:c', 1), ('thread:d', 1)]);
      await data.notifyQuerySynced(hash);
      expect(data.getQueryByHash(hash)!.records.map((r) => r['id']),
          ['thread:d', 'thread:c']);
    });

    test('skips ids that are not resident locally yet', () async {
      seed('thread:c', 'c');
      final hash = await registerPageTwo();
      await data.updateQueryRemoteArray(hash, [('thread:c', 1), ('thread:zz', 1)]);
      await data.notifyQuerySynced(hash);
      expect(data.getQueryByHash(hash)!.records.map((r) => r['id']),
          ['thread:c']);
    });

    test('a non-windowed query still materializes from the circuit array',
        () async {
      seed('thread:a', 'a');
      final hash =
          await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      // A remoteArray must NOT override the circuit's view for an unwindowed
      // query: the circuit is what applies WHERE/ORDER BY there.
      await data.updateQueryRemoteArray(hash, [('thread:a', 1)]);
      await data.notifyQuerySynced(hash);
      expect(data.getQueryByHash(hash)!.records, isEmpty);
    });

    test('records a localFetch timing sample', () async {
      final hash = await registerPageTwo();
      expect(data.phaseStat(hash, TimingPhase.localFetch).count,
          greaterThan(0));
    });

    test('a stream update materializes its own array, not the cached one',
        () async {
      // The cached localArray is still the PREVIOUS view while an update is
      // being processed, so materializing from it would always render one view
      // behind (an inserted row would never appear).
      seed('thread:a', 'a');
      final hash =
          await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      expect(data.getQueryByHash(hash)!.records, isEmpty);

      await data.onStreamUpdate(StreamUpdate(
        queryHash: hash,
        op: 'CREATE',
        localArray: [('thread:a', 1)],
      ));

      expect(data.getQueryByHash(hash)!.records.map((r) => r['id']),
          ['thread:a']);
    });
  });

  group('parseUpdateOptions', () {
    test('no debounce -> empty options', () {
      final o = parseUpdateOptions('thread:a', {'title': 'x'}, null);
      expect(o.debounced, isNull);
    });
    test('recordId key debounces on the id', () {
      final o = parseUpdateOptions(
          'thread:a',
          {'title': 'x'},
          const UpdateOptions(
              debounced: DebounceOptions(key: DebounceKey.recordId)));
      expect(o.debounced!.key, 'thread:a');
      expect(o.debounced!.delay, 200);
    });
    test('recordId_x_fields key includes sorted field names', () {
      final o = parseUpdateOptions(
          'thread:a',
          {'b': 1, 'a': 2},
          const UpdateOptions(
              debounced: DebounceOptions(
                  key: DebounceKey.recordIdXFields, delay: 50)));
      expect(o.debounced!.key, 'thread:a::a#b');
      expect(o.debounced!.delay, 50);
    });
  });
}
