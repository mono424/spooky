import 'dart:async';

import 'modules/auth/auth_service.dart';
import 'modules/cache/cache_module.dart';
import 'modules/data/data_module.dart';
import 'modules/sync/queue/queue_down.dart';
import 'modules/sync/queue/queue_up.dart';
import 'modules/sync/sync.dart';
import 'services/database/local_database_service.dart';
import 'services/database/remote_database_service.dart';
import 'services/logger/logger.dart';
import 'services/persistence/memory_persistence.dart';
import 'services/stream_processor/stream_processor_service.dart';
import 'surreal/remote_client.dart';
import 'surreal/value.dart';
import 'types.dart';

/// Main entry point of the pure-Dart Spooky core (TS `Sp00kyClient`).
///
/// Wires the sqlite local store, the FFI stream processor, the cache, the data
/// module, and (when a remote endpoint is configured) the remote SurrealDB
/// connection, auth, and bidirectional sync. Live queries are exposed as Dart
/// `Stream`s (consume with a Flutter `StreamBuilder`).
class Sp00kyClient {
  Sp00kyClient(this.config,
      {SpookyLogger? logger, RemoteSurrealClient? remoteClient})
      : _logger = logger ?? SpookyLogger.root(),
        _remoteClientOverride = remoteClient;

  final Sp00kyConfig config;
  final SpookyLogger _logger;
  final RemoteSurrealClient? _remoteClientOverride;

  late final LocalDatabaseService _local;
  late final StreamProcessorService _streamProcessor;
  late final CacheModule _cache;
  late final DataModule _dataModule;
  late final PersistenceClient _persistence;

  RemoteDatabaseService? _remote;
  AuthService? _auth;
  Sp00kySync? _sync;

  bool _initialized = false;

  DataModule get dataModule => _dataModule;
  StreamProcessorService get streamProcessor => _streamProcessor;
  LocalDatabaseService get local => _local;

  /// The auth service. Throws if no remote endpoint is configured.
  AuthService get auth {
    final a = _auth;
    if (a == null) throw StateError('Auth requires a remote endpoint');
    return a;
  }

  int get pendingMutationCount => _sync?.pendingMutationCount ?? 0;
  int get liveRetryCount => _sync?.liveRetryCount ?? 0;

  /// Initialize the client. With a remote endpoint configured, also connects
  /// remotely, starts auth, fetches the session id, and starts sync (the JS
  /// `init` sequence). Without one, runs local-first only.
  Future<void> init() async {
    if (_initialized) return;

    _local = LocalDatabaseService.open(_logger, store: config.database.store);
    _local.provision();

    _persistence = _resolvePersistence();
    _streamProcessor = StreamProcessorService(_persistence, _logger);
    await _streamProcessor.init();
    _streamProcessor.seedPermissionsFromSchema(config.schemaSurql);

    _cache = CacheModule(
      _local,
      _streamProcessor,
      (update) => _dataModule.onStreamUpdate(update),
      _logger,
    );

    _dataModule = DataModule(
      _cache,
      _local,
      config.schema,
      _logger,
      streamDebounceTime: config.streamDebounceTime,
    );

    final hasRemote =
        config.database.endpoint != null || _remoteClientOverride != null;

    if (hasRemote) {
      final client = _remoteClientOverride ?? WebSocketSurrealClient();
      final remote = RemoteDatabaseService(config.database, client, _logger);
      await remote.connect();
      _remote = remote;

      final auth = AuthService(config.schema, remote, _persistence, _logger);
      await auth.init();
      _auth = auth;

      final sessionId = await _fetchSessionId();
      await _dataModule.init(sessionId);

      final sync = Sp00kySync(
        _local,
        remote,
        _cache,
        _dataModule,
        config.schema,
        _logger,
        options: Sp00kySyncOptions(refSyncIntervalMs: config.refSyncIntervalMs),
      );
      _sync = sync;

      _setupCallbacks();

      // The synchronous prefix subtlety (TS sp00ky.ts:318-334):
      // setCurrentUserId on DataModule MUST run before the first await so the
      // userQuery's initial fetch sees the right per-user list_ref table.
      auth.subscribe((userId) async {
        _dataModule.setCurrentUserId(userId); // sync, before any await
        final next = await _fetchSessionId();
        _dataModule.setSessionId(next);
        try {
          await sync.setCurrentUserId(userId);
        } catch (e) {
          _logger.error('sync.setCurrentUserId failed', e);
        }
      });

      await sync.init();
    } else {
      // Local-first only: no session salt.
      await _dataModule.init('');
      _setupCallbacks();
    }

    _initialized = true;
    _logger.info('Sp00kyClient initialized');
  }

  void _setupCallbacks() {
    _dataModule.onMutation(_onMutation);
    // CRDT seam (deferred): SYNC_REMOTE_DATA_INGESTED -> crdtManager.applyRow.
    // TODO(crdt): wire when CRDT lands.
  }

  void _onMutation(List<UpEvent> mutations) {
    _sync?.enqueueMutation(mutations);
  }

  /// Fetch `session::id()` for the query-hash salt; empty on failure or no remote.
  Future<String> _fetchSessionId() async {
    final remote = _remote;
    if (remote == null) return '';
    try {
      final result = await remote.query('RETURN <string>session::id()');
      final first = result.isNotEmpty ? result.first : null;
      return first?.toString() ?? '';
    } catch (_) {
      return '';
    }
  }

  // ==================== QUERIES ====================

  /// Register a raw SURQL query and return its hash. The table is parsed from
  /// the first `FROM <table>` (TS `queryRaw`).
  Future<String> queryRaw(
    String sql,
    Map<String, dynamic> params, {
    QueryTimeToLive ttl = defaultTtl,
  }) async {
    final tableName = _parseTableFromSurql(sql);
    final hash = await _dataModule.query(tableName, sql, params, ttl);
    // Trigger remote registration + initial down-sync (TS initQuery).
    _sync?.enqueueDownEvent(RegisterEvent(hash));
    return hash;
  }

  /// Subscribe to a registered query as a broadcast [Stream]. Multiple
  /// listeners share one internal callback registration; [immediate] replays
  /// the current result set on first listen (so a `StreamBuilder` renders
  /// immediately).
  Stream<List<Map<String, dynamic>>> subscribeStream(
    String queryHash, {
    bool immediate = true,
  }) {
    late StreamController<List<Map<String, dynamic>>> controller;
    void Function()? off;
    controller = StreamController<List<Map<String, dynamic>>>.broadcast(
      onListen: () {
        off = _dataModule.subscribe(queryHash, controller.add,
            immediate: immediate);
      },
      onCancel: () {
        off?.call();
        off = null;
      },
    );
    return controller.stream;
  }

  /// Register a query and return a result [Stream] in one call.
  Future<Stream<List<Map<String, dynamic>>>> queryStream(
    String sql,
    Map<String, dynamic> params, {
    QueryTimeToLive ttl = defaultTtl,
  }) async {
    final hash = await queryRaw(sql, params, ttl: ttl);
    return subscribeStream(hash);
  }

  /// Faithful callback-based subscribe (TS `subscribe`).
  void Function() subscribe(
    String hash,
    QueryUpdateCallback callback, {
    bool immediate = false,
  }) =>
      _dataModule.subscribe(hash, callback, immediate: immediate);

  // ==================== MUTATIONS ====================

  Future<Map<String, dynamic>> create(String id, Map<String, dynamic> data) =>
      _dataModule.create(id, data);

  Future<Map<String, dynamic>> update(
    String table,
    String id,
    Map<String, dynamic> data, {
    UpdateOptions? options,
  }) =>
      _dataModule.update(table, id, data, options: options);

  Future<void> delete(String table, String id) => _dataModule.delete(table, id);

  // ==================== CRDT (deferred) ====================

  Future<Never> openCrdtField(String table, String recordId, String field,
          [String? fallbackText]) =>
      throw UnimplementedError('CRDT deferred');

  void closeCrdtField(String table, String recordId, String field) =>
      throw UnimplementedError('CRDT deferred');

  // ==================== LIFECYCLE ====================

  Future<void> close() async {
    await _sync?.close();
    await _remote?.close();
    await _streamProcessor.close();
    _local.close();
    _initialized = false;
  }

  PersistenceClient _resolvePersistence() {
    final pc = config.persistenceClient;
    if (pc is PersistenceClient) return pc;
    return MemoryPersistenceClient();
  }

  /// Parse the target table from a `SELECT ... FROM <table>` query.
  String _parseTableFromSurql(String sql) {
    final match = RegExp(r'\bFROM\s+(?:ONLY\s+)?([A-Za-z_][A-Za-z0-9_]*)',
            caseSensitive: false)
        .firstMatch(sql);
    if (match == null) {
      throw ArgumentError('Could not parse table from query: $sql');
    }
    return match.group(1)!;
  }
}

/// Re-export so callers can build ids without importing the surreal layer.
String recordIdString(String table, Object id) => RecordId(table, id).encode();
