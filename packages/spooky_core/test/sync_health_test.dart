import 'dart:async';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/modules/sync/sync.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/database/remote_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// The idle `_00_list_ref` poll is the only health signal that runs on a quiet
/// client. These tests step `pollListRefOnce()` -> `recordSyncOutcome` directly:
/// a run of network-failed cycles degrades, and a single clean cycle recovers
/// with no mutation. `init()` is never called, so no timers or LIVE start.
void main() {
  final logger = SpookyLogger.root('test');
  const connectionUnavailable =
      'You must be connected to a SurrealDB instance before performing this operation';
  const schemaSurql =
      'DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;';
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };

  late LocalDatabaseService local;
  late StreamProcessorService sp;
  late DataModule data;
  late _HealthRemote remote;
  late Sp00kySync sync;

  /// Register [queries] real queries so the poll has hashes to iterate.
  /// [selfHealBaseMs] defaults high enough to keep the self-heal retry off the
  /// critical path of tests that only step the poll.
  Future<void> setUpSync({
    int queries = 1,
    int degradeAfter = 3,
    int selfHealBaseMs = 3600000,
  }) async {
    local = LocalDatabaseService.open(logger)..provision();
    sp = StreamProcessorService(MemoryPersistenceClient(), logger);
    await sp.init();
    sp.seedPermissionsFromSchema(schemaSurql);
    late DataModule d;
    final cache = CacheModule(local, sp, (u) => d.onStreamUpdate(u), logger);
    d = DataModule(cache, local, schema, logger);
    data = d;
    await data.init('sess');

    remote = _HealthRemote();
    final remoteDb = RemoteDatabaseService(
        const DatabaseConfig(namespace: 't', database: 't'), remote, logger);
    sync = Sp00kySync(
      local,
      remoteDb,
      cache,
      data,
      schema,
      logger,
      options: Sp00kySyncOptions(
        degradeAfterConsecutiveFailures: degradeAfter,
        selfHealBaseMs: selfHealBaseMs,
      ),
    );

    for (var i = 0; i < queries; i++) {
      await data.query('thread', 'SELECT * FROM thread LIMIT ${i + 1}', {}, '10m');
    }
  }

  tearDown(() async {
    await sync.close();
    data.dispose();
    await sp.close();
    local.close();
  });

  test('degrades after N consecutive network-failed poll cycles', () async {
    await setUpSync();
    remote.listRefError = Exception(connectionUnavailable);

    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);
    expect(sync.syncHealth.kind, 'network');
    expect(sync.syncHealth.error, contains(connectionUnavailable));
    expect(sync.syncHealth.consecutiveFailures, 3);
  });

  test('recovers on the next clean poll cycle, no mutation needed', () async {
    await setUpSync();
    remote.listRefError = Exception(connectionUnavailable);
    for (var i = 0; i < 3; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);

    // Connectivity returns; a plain idle poll (no user action) clears it.
    remote.listRefError = null;
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    expect(sync.syncHealth.kind, isNull);
    expect(sync.syncHealth.error, isNull);
  });

  test('does not degrade on application errors (server was reached)', () async {
    await setUpSync();
    remote.listRefError = StateError('Permission denied');
    for (var i = 0; i < 4; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
  });

  test('counts a mixed cycle (one reachable hash) as reached', () async {
    await setUpSync(queries: 2);
    remote.listRefError = Exception(connectionUnavailable);
    for (var i = 0; i < 3; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);

    // One hash answers, the other still network-fails -> reached -> healthy.
    remote.listRefError = null;
    remote.failListRefAfter = 1;
    remote.listRefErrorAfter = Exception(connectionUnavailable);
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
  });

  test('probes RETURN true when there are no active queries', () async {
    await setUpSync(queries: 0);
    remote.probeError = Exception(connectionUnavailable);
    for (var i = 0; i < 3; i++) {
      await sync.pollListRefOnce();
    }
    expect(remote.queries, contains('RETURN true'));
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);

    remote.probeError = null;
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
  });

  test('leaves everConnected false through a cold-start failure run', () async {
    await setUpSync();
    expect(sync.syncHealth.everConnected, isFalse);

    // The server was never reached: three failed cycles degrade, but this is the
    // initial connecting phase, not a lost connection.
    remote.listRefError = Exception(connectionUnavailable);
    for (var i = 0; i < 3; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);
    expect(sync.syncHealth.everConnected, isFalse);
  });

  test('latches everConnected on first success, keeps it through a degrade',
      () async {
    await setUpSync();
    await sync.pollListRefOnce();
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    expect(sync.syncHealth.everConnected, isTrue);

    remote.listRefError = Exception(connectionUnavailable);
    for (var i = 0; i < 3; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.degraded);
    expect(sync.syncHealth.everConnected, isTrue);
  });

  test('degradeAfterConsecutiveFailures 0 disables reporting', () async {
    await setUpSync(degradeAfter: 0);
    remote.listRefError = Exception(connectionUnavailable);
    for (var i = 0; i < 5; i++) {
      await sync.pollListRefOnce();
    }
    expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    expect(sync.syncHealth.consecutiveFailures, 0);
    expect(sync.syncHealth.everConnected, isFalse);
  });

  group('subscribeToSyncHealth', () {
    test('fires immediately, then on each transition only', () async {
      await setUpSync();
      final seen = <SyncHealth>[];
      final off = sync.subscribeToSyncHealth(seen.add);
      expect(seen, hasLength(1)); // immediate snapshot
      expect(seen.single.status, SyncHealthStatus.healthy);

      remote.listRefError = Exception(connectionUnavailable);
      await sync.pollListRefOnce();
      await sync.pollListRefOnce();
      // Below the threshold: no transition, so no emission.
      expect(seen, hasLength(1));

      await sync.pollListRefOnce();
      await _settle();
      expect(seen, hasLength(2));
      expect(seen.last.status, SyncHealthStatus.degraded);

      remote.listRefError = null;
      await sync.pollListRefOnce();
      await _settle();
      expect(seen, hasLength(3));
      expect(seen.last.status, SyncHealthStatus.healthy);

      off();
      remote.listRefError = Exception(connectionUnavailable);
      for (var i = 0; i < 3; i++) {
        await sync.pollListRefOnce();
      }
      await _settle();
      expect(seen, hasLength(3), reason: 'unsubscribed');
    });
  });

  group('self-heal', () {
    test('a degraded client re-probes on its own and recovers', () async {
      // No active queries, so the self-heal path falls through to the direct
      // `RETURN true` connectivity probe. Short backoff so the retry lands
      // inside the test.
      await setUpSync(queries: 0, selfHealBaseMs: 20);
      remote.probeError = Exception(connectionUnavailable);
      for (var i = 0; i < 3; i++) {
        await sync.pollListRefOnce();
      }
      expect(sync.syncHealth.status, SyncHealthStatus.degraded);

      // Connectivity returns. Nothing polls, nothing mutates: only the self-heal
      // timer can flip this back.
      remote.probeError = null;
      await Future<void>.delayed(const Duration(milliseconds: 120));
      expect(sync.syncHealth.status, SyncHealthStatus.healthy);
    });

    test('close() stops the self-heal loop', () async {
      await setUpSync(queries: 0, selfHealBaseMs: 20);
      remote.probeError = Exception(connectionUnavailable);
      for (var i = 0; i < 3; i++) {
        await sync.pollListRefOnce();
      }
      expect(sync.syncHealth.status, SyncHealthStatus.degraded);

      await sync.close();
      final before = remote.queries.length;
      await Future<void>.delayed(const Duration(milliseconds: 60));
      expect(remote.queries.length, before,
          reason: 'no further probes after close');
    });
  });

  group('poll idempotence', () {
    test('an unchanged list_ref does not rewrite the remoteArray', () async {
      await setUpSync();
      remote.listRef = [
        {'out': 'thread:a', 'version': 1}
      ];
      expect(await sync.pollListRefOnce(), isTrue,
          reason: 'the first fetch populates the remoteArray');

      final writesBefore = remote.queries
          .where((q) => q.contains('UPDATE') || q.contains('_00_query'))
          .length;
      expect(await sync.pollListRefOnce(), isFalse,
          reason: 'nothing changed on the second cycle');
      final writesAfter = remote.queries
          .where((q) => q.contains('UPDATE') || q.contains('_00_query'))
          .length;
      expect(writesAfter, writesBefore);
    });

    test('a changed list_ref reports changed', () async {
      await setUpSync();
      await sync.pollListRefOnce();
      remote.listRef = [
        {'out': 'thread:a', 'version': 1}
      ];
      expect(await sync.pollListRefOnce(), isTrue);
    });
  });
}

Future<void> _settle() => Future<void>.delayed(const Duration(milliseconds: 20));

/// A [RemoteSurrealClient] whose `_00_list_ref` select and `RETURN true` probe
/// can be made to fail on demand, so a poll cycle's reachability is scripted.
class _HealthRemote implements RemoteSurrealClient {
  final List<String> queries = [];

  /// Rows the list_ref select returns.
  List<Map<String, dynamic>> listRef = [];

  /// Thrown by every list_ref select while set.
  Object? listRefError;

  /// After this many successful list_ref selects in one cycle, throw
  /// [listRefErrorAfter] instead. Lets a cycle be partly reachable.
  int? failListRefAfter;
  Object? listRefErrorAfter;
  int _listRefCalls = 0;

  /// Thrown by the `RETURN true` connectivity probe while set.
  Object? probeError;

  final _connected = StreamController<void>.broadcast();
  final _disconnected = StreamController<void>.broadcast();

  @override
  Stream<void> get onConnected => _connected.stream;
  @override
  Stream<void> get onDisconnected => _disconnected.stream;

  @override
  Future<void> connect(String endpoint) async {}
  @override
  Future<void> use(
      {required String namespace, required String database}) async {}
  @override
  Future<dynamic> authenticate(String token) async => null;
  @override
  Future<dynamic> signin(Map<String, dynamic> params) async => {'access': 'tok'};
  @override
  Future<dynamic> signup(Map<String, dynamic> params) async => {'access': 'tok'};
  @override
  Future<void> invalidate() async {}

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    queries.add(sql);
    if (sql == 'RETURN true') {
      if (probeError != null) throw probeError!;
      return [true];
    }
    if (sql.contains('FROM _00_list_ref')) {
      if (listRefError != null) throw listRefError!;
      final n = _listRefCalls++;
      final threshold = failListRefAfter;
      if (threshold != null && n >= threshold && listRefErrorAfter != null) {
        throw listRefErrorAfter!;
      }
      return [listRef];
    }
    if (sql.contains(r'FROM $idsToFetch')) return [<dynamic>[]];
    return [null];
  }

  @override
  Future<(String, Stream<LiveMessage>)> live(String sql,
          [Map<String, dynamic>? vars]) async =>
      ('live-1', const Stream<LiveMessage>.empty());

  @override
  Future<void> kill(String liveId) async {}

  @override
  Future<void> close() async {
    await _connected.close();
    await _disconnected.close();
  }
}
