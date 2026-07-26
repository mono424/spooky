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

  /// A second DataModule over the same stores, standing in for local-only mode
  /// (no remote registration, so queries are not born `fetching`).
  Future<DataModule> localOnlyModule() async {
    late DataModule d;
    final cache = CacheModule(local, sp, (u) => d.onStreamUpdate(u), logger);
    d = DataModule(cache, local, schema, logger, bornFetching: false);
    await d.init('sess-local');
    return d;
  }

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
    test('a fresh query is born fetching and replays that immediately',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add, immediate: true);
      expect(seen, [QueryStatus.fetching]);
    });

    test('in local-only mode a fresh query starts idle', () async {
      final localOnly = await localOnlyModule();
      final hash =
          await localOnly.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      localOnly.subscribeStatus(hash, seen.add, immediate: true);
      expect(seen, [QueryStatus.idle]);
      localOnly.dispose();
    });

    test('immediate replay before registration defaults to idle', () {
      final seen = <QueryStatus>[];
      data.subscribeStatus('unknown-hash', seen.add, immediate: true);
      expect(seen, [QueryStatus.idle]);
    });

    test('setQueryStatus notifies on fetching -> idle -> fetching', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);
      data.setQueryStatus(hash, QueryStatus.idle);
      data.setQueryStatus(hash, QueryStatus.fetching);
      expect(seen, [QueryStatus.idle, QueryStatus.fetching]);
    });

    test('setQueryStatus is a no-op when the status is unchanged', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);
      data.setQueryStatus(hash, QueryStatus.fetching); // already fetching
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
      data.setQueryStatus(hash, QueryStatus.idle);
      off();
      data.setQueryStatus(hash, QueryStatus.fetching);
      expect(seen, [QueryStatus.idle]);
    });
  });

  group('refcounted fetch cycles', () {
    test('only the outermost cycle flips the status', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      data.setQueryStatus(hash, QueryStatus.idle); // settle the born state
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);

      data.beginFetching(hash);
      data.beginFetching(hash); // nested round on the same hash
      expect(seen, [QueryStatus.fetching]);

      data.endFetching(hash); // inner exit: still fetching
      expect(seen, [QueryStatus.fetching]);

      data.endFetching(hash); // outermost exit settles it
      expect(seen, [QueryStatus.fetching, QueryStatus.idle]);
    });

    test('an unbalanced end settles the query and floors the depth', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash, seen.add);

      // Born fetching, so a stray end settles it rather than going negative.
      data.endFetching(hash);
      data.endFetching(hash);
      expect(seen, [QueryStatus.idle]);

      // The depth floored at 0, so the next begin still emits.
      data.beginFetching(hash);
      expect(seen, [QueryStatus.idle, QueryStatus.fetching]);
    });

    test('finalizeDeregister drops the depth so a re-register starts clean',
        () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      data.beginFetching(hash); // leaked cycle, never ended
      data.finalizeDeregister(hash);

      final hash2 =
          await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      expect(hash2, hash);
      final seen = <QueryStatus>[];
      data.subscribeStatus(hash2, seen.add);
      data.endFetching(hash2);
      expect(seen, [QueryStatus.idle], reason: 'depth did not survive teardown');
    });
  });

  group('notifyQuerySynced', () {
    test('emits once for an empty result set, then only on change', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final emissions = <List<Map<String, dynamic>>>[];
      data.subscribe(hash, emissions.add);

      await data.notifyQuerySynced(hash);
      expect(emissions, hasLength(1));
      expect(emissions.single, isEmpty);

      // Nothing changed and it already notified: no second emission.
      await data.notifyQuerySynced(hash);
      expect(emissions, hasLength(1));
    });

    test('a re-registered query re-emits even when unchanged', () async {
      final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      await data.notifyQuerySynced(hash);
      // Tear down and re-register: `updateCount` survives in `_00_query`, but the
      // ephemeral syncNotified flag must not, else the new subscriber never gets
      // an emission and stays loading forever.
      data.finalizeDeregister(hash);

      final hash2 =
          await data.query('thread', 'SELECT * FROM thread', {}, '10m');
      final emissions = <List<Map<String, dynamic>>>[];
      data.subscribe(hash2, emissions.add);
      await data.notifyQuerySynced(hash2);
      expect(emissions, hasLength(1));
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
