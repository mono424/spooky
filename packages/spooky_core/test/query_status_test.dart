import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// Covers the query fetch-status API (`subscribeStatus` / `setQueryStatus`) and
/// the two-phase `deregisterQuery` / `finalizeDeregister` teardown ported from
/// the TS core.
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
  final deregistered = <String>[];

  setUp(() async {
    deregistered.clear();
    local = LocalDatabaseService.open(logger)..provision();
    sp = StreamProcessorService(MemoryPersistenceClient(), logger);
    await sp.init();
    sp.seedPermissionsFromSchema(
        'DEFINE TABLE thread PERMISSIONS FOR select WHERE true;');
    late DataModule d;
    final cache = CacheModule(local, sp, (u) => d.onStreamUpdate(u), logger);
    d = DataModule(cache, local, schema, logger, onDeregister: deregistered.add);
    data = d;
    await data.init('sess');
  });
  tearDown(() async {
    data.dispose();
    await sp.close();
    local.close();
  });

  group('query fetch status', () {
    test('subscribeStatus with immediate replays the current status (idle)',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add, immediate: true);
      expect(seen, [QueryStatus.idle]);
    });

    test('immediate replay before registration defaults to idle', () {
      final seen = <QueryStatus>[];
      data.subscribeStatus('unknown-hash', seen.add, immediate: true);
      expect(seen, [QueryStatus.idle]);
    });

    test('setQueryStatus notifies on idle -> fetching -> idle', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);
      data.setQueryStatus(hash, QueryStatus.fetching);
      data.setQueryStatus(hash, QueryStatus.idle);
      expect(seen, [QueryStatus.fetching, QueryStatus.idle]);
    });

    test('setQueryStatus is a no-op when the status is unchanged', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);
      data.setQueryStatus(hash, QueryStatus.idle); // already idle
      expect(seen, isEmpty);
    });

    test('setQueryStatus is a no-op for an unknown query', () {
      final seen = <QueryStatus>[];
      data.subscribeStatus('nope', seen.add);
      data.setQueryStatus('nope', QueryStatus.fetching);
      expect(seen, isEmpty);
    });

    test('unsubscribe stops further status notifications', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      final off = data.subscribeStatus(hash, seen.add);
      data.setQueryStatus(hash, QueryStatus.fetching);
      off();
      data.setQueryStatus(hash, QueryStatus.idle);
      expect(seen, [QueryStatus.fetching]);
    });
  });

  group('deregisterQuery', () {
    test('fires the onDeregister hook when no subscribers remain', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      data.deregisterQuery(hash);
      expect(deregistered, [hash]);
    });

    test('is a no-op while a subscriber remains (refcount)', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      data.subscribe(hash, (_) {});
      data.deregisterQuery(hash);
      expect(deregistered, isEmpty);
    });

    test('is a no-op for an unknown query', () {
      data.deregisterQuery('unknown');
      expect(deregistered, isEmpty);
    });

    test('finalizeDeregister removes the query from the active set', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      expect(data.getActiveQueryHashes(), contains(hash));
      data.finalizeDeregister(hash);
      expect(data.getActiveQueryHashes(), isNot(contains(hash)));
    });

    test('status subscriptions are dropped on finalizeDeregister', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);
      data.finalizeDeregister(hash);
      // The query is gone, so setQueryStatus is now a no-op.
      data.setQueryStatus(hash, QueryStatus.fetching);
      expect(seen, isEmpty);
    });
  });
}
