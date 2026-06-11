import 'dart:async';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/sync/engine.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/database/remote_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// Fake remote returning canned rows for `SELECT * FROM $idsToFetch` and the
/// existence check used by handleRemovedRecords.
class FakeEngineRemote implements RemoteSurrealClient {
  final Map<String, Map<String, dynamic>> records = {};
  final Set<String> existing = {};

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    // Record fetch: `SELECT * FROM $idsToFetch`. ($idsToFetch contains $ids as a
    // substring, so check it first.)
    if (sql.contains(r'$idsToFetch')) {
      final ids = (vars?['idsToFetch'] as List?) ?? const [];
      return [
        ids
            .map((id) => records[id.toString()])
            .where((r) => r != null)
            .toList(),
      ];
    }
    // Existence check: `SELECT id FROM $ids`.
    if (sql.contains(r'$ids')) {
      final ids = (vars?['ids'] as List?) ?? const [];
      return [
        ids
            .map((id) => id.toString())
            .where(existing.contains)
            .map((id) => {'id': id})
            .toList(),
      ];
    }
    return [null];
  }

  @override
  Future<void> connect(String e) async {}
  @override
  Future<void> use(
      {required String namespace, required String database}) async {}
  @override
  Future<dynamic> authenticate(String t) async {}
  @override
  Future<dynamic> signin(Map<String, dynamic> p) async {}
  @override
  Future<dynamic> signup(Map<String, dynamic> p) async {}
  @override
  Future<void> invalidate() async {}
  @override
  Future<(String, Stream<LiveMessage>)> live(String s,
          [Map<String, dynamic>? v]) async =>
      ('l', const Stream<LiveMessage>.empty());
  @override
  Future<void> kill(String id) async {}
  @override
  Stream<void> get onConnected => const Stream.empty();
  @override
  Stream<void> get onDisconnected => const Stream.empty();
  @override
  Future<void> close() async {}
}

void main() {
  final logger = SpookyLogger.root('test');
  final schema = {
    'thread': {
      'columns': {'title': const ColumnSchema(type: 'string')},
    },
  };

  late LocalDatabaseService local;
  late StreamProcessorService sp;
  late CacheModule cache;
  late FakeEngineRemote remoteClient;
  late SyncEngine engine;

  setUp(() async {
    local = LocalDatabaseService.open(logger)..provision();
    sp = StreamProcessorService(MemoryPersistenceClient(), logger);
    await sp.init();
    sp.seedPermissionsFromSchema(
        'DEFINE TABLE thread PERMISSIONS FOR select WHERE true;');
    cache = CacheModule(local, sp, (_) {}, logger);
    remoteClient = FakeEngineRemote();
    final remote = RemoteDatabaseService(
        const DatabaseConfig(namespace: 'n', database: 'd'),
        remoteClient,
        logger);
    engine = SyncEngine(remote, cache, schema, logger);
  });
  tearDown(() async {
    await sp.close();
    local.close();
  });

  ({RecordId id, int version}) item(String id, int v) =>
      (id: RecordId.parse(id), version: v);

  test('added records are fetched, cleaned, and saved locally', () async {
    remoteClient.records['thread:a'] = {
      'id': 'thread:a',
      'title': 'hi',
      'server_only': 'x', // stripped by cleanRecord
    };
    await engine.syncRecords(RecordVersionDiff(
        added: [item('thread:a', 1)], updated: [], removed: []));

    final saved = local.getById('thread:a');
    expect(saved, isNotNull);
    expect(saved!['title'], 'hi');
    expect(saved.containsKey('server_only'), isFalse); // cleaned
    expect(cache.lookup('thread:a'), 1);
  });

  test('a record whose remote version <= local is skipped', () async {
    // Seed local at version 5.
    await cache.saveBatch([
      CacheRecord(
          table: 'thread',
          op: 'CREATE',
          record: {'id': 'thread:a', 'title': 'local'},
          version: 5),
    ]);
    remoteClient.records['thread:a'] = {'id': 'thread:a', 'title': 'stale'};
    // Diff says version 3 (older than local 5) -> skip.
    await engine.syncRecords(RecordVersionDiff(
        added: [], updated: [item('thread:a', 3)], removed: []));
    expect(local.getById('thread:a')!['title'], 'local'); // unchanged
  });

  test('removed records are deleted only when gone upstream', () async {
    await cache.saveBatch([
      CacheRecord(
          table: 'thread',
          op: 'CREATE',
          record: {'id': 'thread:keep', 'title': 'k'},
          version: 1),
      CacheRecord(
          table: 'thread',
          op: 'CREATE',
          record: {'id': 'thread:gone', 'title': 'g'},
          version: 1),
    ]);
    // Upstream still has thread:keep but not thread:gone.
    remoteClient.existing.add('thread:keep');

    await engine.syncRecords(RecordVersionDiff(
        added: [],
        updated: [],
        removed: [
          RecordId.parse('thread:keep'),
          RecordId.parse('thread:gone')
        ]));

    expect(
        local.getById('thread:keep'), isNotNull); // verified present upstream
    expect(local.getById('thread:gone'), isNull); // confirmed removed
  });

  test('empty diff is a no-op', () async {
    await engine
        .syncRecords(RecordVersionDiff(added: [], updated: [], removed: []));
    expect(local.getAll('thread'), isEmpty);
  });
}
