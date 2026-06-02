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
