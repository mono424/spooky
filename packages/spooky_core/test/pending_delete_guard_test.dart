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

/// A record deleted locally must not be resurrected by the next sync round. The
/// remote delete rides the outbox, so until it is processed the server's
/// `_00_list_ref` still lists the record and the diff classifies it as `added`.
/// `runSyncForQuery` drops ids with a pending local DELETE from the re-add paths.
void main() {
  final logger = SpookyLogger.root('test');
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
  late _Remote remote;
  late Sp00kySync sync;

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

    remote = _Remote();
    sync = Sp00kySync(
      local,
      RemoteDatabaseService(
          const DatabaseConfig(namespace: 't', database: 't'), remote, logger),
      cache,
      data,
      schema,
      logger,
    );
  });
  tearDown(() async {
    await sync.close();
    data.dispose();
    await sp.close();
    local.close();
  });

  test('a pending local delete suppresses the re-add', () async {
    remote.records['thread:a'] = {
      'id': 'thread:a',
      'title': 'doomed',
      '_00_rv': 1
    };
    final hash =
        await data.query('thread', 'SELECT * FROM thread', {}, '10m');

    // The server still lists the record in the query's window.
    await data.updateQueryRemoteArray(hash, [('thread:a', 1)]);
    // ...and the user's delete is still sitting in the outbox.
    local.putMutation('_00_pending_mutations:1', {
      'mutationType': 'delete',
      'recordId': 'thread:a',
      'created_at': DateTime.now().toUtc().toIso8601String(),
    });

    await sync.syncQuery(hash);
    await Future<void>.delayed(const Duration(milliseconds: 20));

    expect(local.getById('thread:a'), isNull,
        reason: 'the record must not come back while its delete is pending');
    expect(remote.fetchedIds, isEmpty,
        reason: 'the suppressed id should not even be fetched');
  });

  test('once the outbox is clear, the same round re-adds the record', () async {
    remote.records['thread:a'] = {
      'id': 'thread:a',
      'title': 'alive',
      '_00_rv': 1
    };
    final hash =
        await data.query('thread', 'SELECT * FROM thread', {}, '10m');
    await data.updateQueryRemoteArray(hash, [('thread:a', 1)]);

    await sync.syncQuery(hash);
    await Future<void>.delayed(const Duration(milliseconds: 20));

    expect(local.getById('thread:a'), isNotNull);
    expect(remote.fetchedIds, contains('thread:a'));
  });

  test('other ids in the same diff still sync', () async {
    remote.records['thread:a'] = {'id': 'thread:a', 'title': 'x', '_00_rv': 1};
    remote.records['thread:b'] = {'id': 'thread:b', 'title': 'y', '_00_rv': 1};
    final hash =
        await data.query('thread', 'SELECT * FROM thread', {}, '10m');
    await data
        .updateQueryRemoteArray(hash, [('thread:a', 1), ('thread:b', 1)]);
    local.putMutation('_00_pending_mutations:1', {
      'mutationType': 'delete',
      'recordId': 'thread:a',
      'created_at': DateTime.now().toUtc().toIso8601String(),
    });

    await sync.syncQuery(hash);
    await Future<void>.delayed(const Duration(milliseconds: 20));

    expect(local.getById('thread:a'), isNull);
    expect(local.getById('thread:b'), isNotNull);
  });
}

/// Serves record bodies and records which ids were asked for.
class _Remote implements RemoteSurrealClient {
  final Map<String, Map<String, dynamic>> records = {};
  final List<String> fetchedIds = [];

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    if (sql.contains(r'$idsToFetch')) {
      final ids = (vars?['idsToFetch'] as List?) ?? const [];
      fetchedIds.addAll(ids.map((id) => id.toString()));
      return [
        ids
            .map((id) => records[id.toString()])
            .where((r) => r != null)
            .toList(),
      ];
    }
    if (sql.contains(r'$ids')) return [<dynamic>[]];
    return [null];
  }

  @override
  Future<void> connect(String endpoint) async {}
  @override
  Future<void> use(
      {required String namespace, required String database}) async {}
  @override
  Future<dynamic> authenticate(String token) async => null;
  @override
  Future<dynamic> signin(Map<String, dynamic> p) async => {'access': 't'};
  @override
  Future<dynamic> signup(Map<String, dynamic> p) async => {'access': 't'};
  @override
  Future<void> invalidate() async {}
  @override
  Future<(String, Stream<LiveMessage>)> live(String sql,
          [Map<String, dynamic>? vars]) async =>
      ('l', const Stream<LiveMessage>.empty());
  @override
  Future<void> kill(String liveId) async {}
  @override
  Stream<void> get onConnected => const Stream.empty();
  @override
  Stream<void> get onDisconnected => const Stream.empty();
  @override
  Future<void> close() async {}
}
