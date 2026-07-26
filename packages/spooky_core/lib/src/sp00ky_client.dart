import 'dart:async';

import 'modules/app_release/app_release.dart';
import 'modules/auth/auth_service.dart';
import 'modules/bucket.dart';
import 'modules/cache/cache_module.dart';
import 'modules/data/data_module.dart';
import 'modules/feature_flag/feature_flag.dart';
import 'modules/query_builder.dart';
import 'modules/sync/queue/queue_down.dart';
import 'modules/sync/queue/queue_up.dart';
import 'modules/sync/sync.dart';
import 'services/database/local_database_service.dart';
import 'services/database/local_migrator.dart';
import 'services/database/remote_database_service.dart';
import 'services/logger/logger.dart';
import 'services/persistence/memory_persistence.dart';
import 'services/persistence/sqlite_persistence.dart';
import 'services/stream_processor/stream_processor_service.dart';
import 'surreal/remote_client.dart';
import 'surreal/value.dart';
import 'types.dart';
import 'utils/duration_utils.dart';
import 'utils/parser.dart';

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
  FeatureFlagModule? _featureFlags;
  AppReleaseModule? _appReleases;

  /// Query hashes already prewarmed this session, so a repeated [preload] of the
  /// same query is free.
  final Set<String> _preloadedHashes = {};

  /// In-flight background init chains (hydrate + register) per query hash, so
  /// concurrent registrations of one query share a single chain.
  final Map<String, Future<void>> _pendingQueryInits = {};

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

  /// Subscribe to the pending-mutation count (TS `subscribeToPendingMutations`).
  /// Returns an unsubscribe fn; a no-op unsubscribe in local-only mode.
  void Function() subscribeToPendingMutations(void Function(int count) cb) =>
      _sync?.subscribeToPendingMutations(cb) ?? () {};

  /// Current sync-health snapshot (TS `syncHealth`). A local-only client is
  /// always healthy: there is nothing to reach.
  SyncHealth get syncHealth =>
      _sync?.syncHealth ??
      const SyncHealth(
        status: SyncHealthStatus.healthy,
        consecutiveFailures: 0,
        everConnected: true,
      );

  /// Observe sync health (TS `subscribeToSyncHealth`). Fires immediately with
  /// the current snapshot and again on every healthy<->degraded transition.
  /// Returns an unsubscribe fn. In local-only mode fires once with the healthy
  /// snapshot and never again.
  void Function() subscribeToSyncHealth(void Function(SyncHealth health) cb) {
    final sync = _sync;
    if (sync == null) {
      cb(syncHealth);
      return () {};
    }
    return sync.subscribeToSyncHealth(cb);
  }

  /// Sync health as a broadcast [Stream], for `StreamBuilder` friendliness
  /// (consistent with [subscribeStream]). Replays the current snapshot on first
  /// listen.
  Stream<SyncHealth> syncHealthStream() {
    late StreamController<SyncHealth> controller;
    void Function()? off;
    controller = StreamController<SyncHealth>.broadcast(
      onListen: () => off = subscribeToSyncHealth(controller.add),
      onCancel: () {
        off?.call();
        off = null;
      },
    );
    return controller.stream;
  }

  /// Initialize the client. With a remote endpoint configured, also connects
  /// remotely, starts auth, fetches the session id, and starts sync (the JS
  /// `init` sequence). Without one, runs local-first only.
  Future<void> init() async {
    if (_initialized) return;

    _local = LocalDatabaseService.open(_logger,
        store: config.database.store, path: config.database.localDbPath);
    _local.provision();

    // Migrate before the stream processor loads state, so a schema change
    // wipes stale circuit state instead of restoring views over old tables.
    await LocalMigrator(_local, _logger).provision(config.schemaSurql);

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

    final hasRemote =
        config.database.endpoint != null || _remoteClientOverride != null;

    _dataModule = DataModule(
      _cache,
      _local,
      config.schema,
      _logger,
      streamDebounceTime: config.streamDebounceTime,
      // Only start queries `fetching` when a remote registration will settle
      // them; a local-only query has no such lifecycle.
      bornFetching: hasRemote,
      // Keep the server-side registration alive: re-register on each TTL beat.
      // Reads `_sync` at fire-time (set later in init); no-op in local-only.
      onHeartbeat: (hash) => _sync?.enqueueDownEvent(HeartbeatEvent(hash)),
      // Opt-in query teardown: enqueue the remote `_00_query` cleanup; the local
      // view + state are freed by sync after the remote delete. No-op when local.
      onDeregister: (hash) => _sync?.enqueueDownEvent(CleanupEvent(hash)),
    );

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
        options: Sp00kySyncOptions(
          refSyncIntervalMs: config.refSyncIntervalMs,
          anonymousLiveQueries: config.enableAnonymousLiveQueries,
          // A null syncHealth config disables degraded reporting, matching the
          // TS `syncHealth: false`.
          degradeAfterConsecutiveFailures:
              config.syncHealth?.degradeAfterConsecutiveFailures ?? 0,
        ),
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

      // Reactive feature flags: a shared live query over the user's
      // `_00_user_feature` assignments (TS `FeatureFlagModule`). init() subscribes
      // to auth so the query follows sign-in/out.
      final featureFlags = FeatureFlagModule(
        dataModule: _dataModule,
        sync: sync,
        auth: auth,
        logger: _logger,
      );
      featureFlags.init();
      _featureFlags = featureFlags;

      // Release announcements: a shared live query over the world-readable
      // `_00_app_release` rows, so an app can prompt (or force) an update when
      // the deployed version moves past the running build.
      final appReleases = AppReleaseModule(
        dataModule: _dataModule,
        sync: sync,
        auth: auth,
        logger: _logger,
      );
      appReleases.init();
      _appReleases = appReleases;
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
    } catch (err) {
      // session::id() is only a salt for query-hash isolation; if the remote
      // round-trip fails, fall back to an empty salt rather than failing init.
      _logger.debug('session::id() fetch failed; using empty salt: $err');
      return '';
    }
  }

  // ==================== QUERIES ====================

  /// One-shot direct remote query, bypassing the local sync layer (TS
  /// `useRemote(r => r.query(...))`). Returns the per-statement results.
  /// Throws if no remote endpoint is configured.
  Future<List<dynamic>> queryRemote(String sql,
      [Map<String, dynamic>? vars]) {
    final remote = _remote;
    if (remote == null) {
      throw StateError('queryRemote requires a remote endpoint');
    }
    return remote.query(sql, vars);
  }

  /// Register a raw SURQL query and return its hash. The table is parsed from
  /// the first `FROM <table>` (TS `queryRaw` / `initQuery`).
  ///
  /// Local-first paint: the hash comes back as soon as the LOCAL registration
  /// completes, with `records` already seeded from the local cache, so a
  /// `StreamBuilder` paints from memory with no network on the paint path.
  /// Instant-hydrate (opt-in) and the `register` down-event continue in a
  /// background chain — hydrate strictly BEFORE the enqueue, so a stale one-shot
  /// snapshot can never land after sync's authoritative `_00_list_ref` overwrite.
  /// Concurrent registrations of the same query share one chain; a sequential
  /// re-registration starts a fresh one, so its `register` keeps freshening warm
  /// data on use.
  Future<String> queryRaw(
    String sql,
    Map<String, dynamic> params, {
    QueryTimeToLive ttl = defaultTtl,
    List<RelationPlan> relations = const [],
  }) async {
    final tableName = _parseTableFromSurql(sql);
    final hash = await _dataModule
        .query(tableName, sql, params, ttl, relations: relations);

    if (!_pendingQueryInits.containsKey(hash)) {
      _pendingQueryInits[hash] = _finishQueryInit(hash, sql, params)
          .whenComplete(() => _pendingQueryInits.remove(hash));
    }
    return hash;
  }

  /// Background tail of [queryRaw]: opt-in instant-hydrate for a cold query,
  /// then the `register` down-event (TS `finishQueryInit`). Never throws — both
  /// halves catch and log, so an unawaited chain can't surface an unhandled async
  /// error. No-op in local-only mode: there is nothing to hydrate from and
  /// nothing to register.
  Future<void> _finishQueryInit(
    String hash,
    String sql,
    Map<String, dynamic> params,
  ) async {
    final sync = _sync;
    if (sync == null) return;

    if (config.instantHydrate && _dataModule.isCold(hash)) {
      try {
        final results = await queryRemote(sql, params);
        final rows = _firstRowsAsMaps(results);
        await _dataModule.applyHydration(hash, rows);
      } catch (err) {
        _logger.warn('Instant hydrate failed; proceeding with registration: $err');
      }
    }

    try {
      sync.enqueueDownEvent(RegisterEvent(hash));
    } catch (err) {
      _logger.error('Failed to enqueue register down-event', err);
    }
  }

  /// Smart, awaitable prewarm into the LOCAL cache without registering a live
  /// view (no `_00_query`, no subscription, no TTL heartbeat) (TS `preload`).
  ///
  /// Cache-aware via a durable freshness marker (`_00_preload`):
  /// - COLD (never preloaded): fetch the query one-shot, persist the rows, stamp
  ///   the marker — and AWAIT it. Callers can `await client.preload(...)` to hold
  ///   the UI until the data is ready.
  /// - WARM (marker present): returns instantly, NEVER blocks.
  ///   [PreloadOptions.refresh] decides whether to also kick a one-time silent
  ///   refetch; the default [PreloadRefresh.onUse] does nothing and the data
  ///   freshens when the real query mounts.
  ///
  /// Best-effort: a fetch failure is a logged no-op that writes no marker, so it
  /// is retried next time. Deduped per session by query hash. Requires a remote
  /// endpoint.
  Future<void> preload(
    String sql,
    Map<String, dynamic> params, {
    PreloadOptions options = const PreloadOptions(),
  }) async {
    if (_remote == null) {
      throw StateError('preload() requires a remote endpoint');
    }
    final tableName = _parseTableFromSurql(sql);
    final hash = _dataModule.calculateHash({'surql': sql, 'params': params});
    if (_preloadedHashes.contains(hash)) return;

    final marker = _dataModule.getPreloadMarker(hash);

    // COLD -> fetch + persist + stamp, awaited so the caller can block on it.
    if (marker == null) {
      final rowCount = await _fetchAndPersist(sql, params, tableName);
      if (rowCount >= 0) {
        _dataModule.writePreloadMarker(hash, rowCount);
        _preloadedHashes.add(hash);
      }
      return;
    }

    // WARM -> never block. Mark handled for this session, then maybe refresh.
    _preloadedHashes.add(hash);
    switch (options.refresh) {
      case PreloadRefresh.onUse:
        return;
      case PreloadRefresh.stale:
        final age = DateTime.now().millisecondsSinceEpoch - marker.fetchedAt;
        if (age <= parseDuration(options.staleTime)) return; // still fresh
      case PreloadRefresh.background:
        break;
    }

    unawaited(_fetchAndPersist(sql, params, tableName).then((rowCount) {
      if (rowCount >= 0) _dataModule.writePreloadMarker(hash, rowCount);
    }));
  }

  /// One-shot remote fetch + local persist for a preload query. Returns the row
  /// count, or -1 on failure (logged, never thrown) so the caller skips stamping
  /// the freshness marker and retries next time.
  Future<int> _fetchAndPersist(
    String sql,
    Map<String, dynamic> params,
    String tableName,
  ) async {
    try {
      final rows = _firstRowsAsMaps(await queryRemote(sql, params));
      await _dataModule.persistSnapshot(tableName, rows);
      return rows.length;
    } catch (err) {
      _logger
          .warn('Preload fetch failed; data will be fetched on demand: $err');
      return -1;
    }
  }

  /// First statement's rows from a remote result, as records with ids.
  List<Map<String, dynamic>> _firstRowsAsMaps(List<dynamic> results) => [
        for (final row in firstRows(results))
          if (row is Map && row['id'] != null) row.cast<String, dynamic>(),
      ];

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

  /// A fluent query builder for [table]. Call `.stream()` / `.run()` to
  /// register and subscribe (TS `query`).
  QueryBuilder query(String table) => QueryBuilder(
        table,
        registrar: (sql, vars, ttl, relations) =>
            queryRaw(sql, vars, ttl: ttl, relations: relations),
        subscriber: subscribeStream,
        // The schema carries the `relationships` codegen emits, which is what
        // resolves a `.related()` field to its table, cardinality and FK.
        schema: config.schema,
        logger: _logger,
      );

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

  /// Subscribe to a query's fetch status (idle/fetching) via callback (TS
  /// `subscribeQueryStatus`). With [immediate] the callback fires synchronously
  /// with the current status. Returns an unsubscribe fn.
  void Function() subscribeQueryStatus(
    String queryHash,
    QueryStatusCallback callback, {
    bool immediate = false,
  }) =>
      _dataModule.subscribeStatus(queryHash, callback, immediate: immediate);

  /// A query's fetch status (idle/fetching) as a broadcast [Stream], for
  /// `StreamBuilder` friendliness (consistent with [subscribeStream]).
  /// [immediate] replays the current status on first listen.
  Stream<QueryStatus> queryStatusStream(
    String queryHash, {
    bool immediate = true,
  }) {
    late StreamController<QueryStatus> controller;
    void Function()? off;
    controller = StreamController<QueryStatus>.broadcast(
      onListen: () {
        off = _dataModule.subscribeStatus(queryHash, controller.add,
            immediate: immediate);
      },
      onCancel: () {
        off?.call();
        off = null;
      },
    );
    return controller.stream;
  }

  /// Report the UI reconcile time (ms) for a query (TS
  /// `reportFrontendTiming`). Call this after applying an update in a widget to
  /// attribute build/paint time to the query in its timing breakdown; read it
  /// back with `dataModule.phaseStat(hash, TimingPhase.frontend)`.
  void reportFrontendTiming(String queryHash, double ms) =>
      _dataModule.recordFrontendTiming(queryHash, ms);

  /// Opt-in eager teardown of a query whose last subscriber has left (TS
  /// `deregisterQuery`): enqueues the remote `_00_query` cleanup and frees the
  /// local view once it completes. No-op while any subscriber remains. Most
  /// queries should NOT call this — the default keep-alive avoids
  /// re-registration churn on navigation.
  void deregisterQuery(String queryHash) =>
      _dataModule.deregisterQuery(queryHash);

  // ==================== AUTH ====================

  /// Authenticate the remote connection with a raw token (TS `authenticate`).
  /// Bypasses [AuthService]: use this when a token comes from outside the
  /// client (e.g. restored by the host app). Throws without a remote endpoint.
  Future<dynamic> authenticate(String token) {
    final remote = _remote;
    if (remote == null) {
      throw StateError('authenticate() requires a remote endpoint');
    }
    return remote.authenticate(token);
  }

  /// Invalidate the remote session (TS `deauthenticate`). Throws without a
  /// remote endpoint.
  Future<void> deauthenticate() {
    final remote = _remote;
    if (remote == null) {
      throw StateError('deauthenticate() requires a remote endpoint');
    }
    return remote.invalidate();
  }

  // ==================== FEATURE FLAGS ====================

  /// A reactive handle for feature flag [key] (TS `feature`). Reads the
  /// signed-in user's `_00_user_feature` assignment over the shared live query;
  /// an unassigned key resolves to [fallback]. Use `.variant()`, `.enabled()`,
  /// `.payload<T>()`, or `.subscribe(cb)`; call `.close()` when done.
  ///
  /// Requires a remote endpoint (feature flags are server-assigned). Throws in
  /// local-only mode.
  FeatureFlagHandle feature(
    String key, {
    String? fallback,
    QueryTimeToLive? ttl,
  }) {
    final ff = _featureFlags;
    if (ff == null) {
      throw StateError('feature() requires a remote endpoint');
    }
    return ff.feature(key, fallback: fallback, ttl: ttl);
  }

  // ==================== APP RELEASES ====================

  /// A reactive handle for app [app]'s latest announced release (TS
  /// `appRelease`). Reads the world-readable `_00_app_release` row over the
  /// shared live query; an app with no row reports no update. Compare against
  /// the running build with `.updateAvailable(currentVersion)`, and read
  /// `.mandatory` / `.cacheBust` to decide how to apply it. Call `.close()` when
  /// done.
  ///
  /// Requires a remote endpoint (releases are server-announced). Throws in
  /// local-only mode.
  AppReleaseHandle appRelease(String app, {QueryTimeToLive? ttl}) {
    final releases = _appReleases;
    if (releases == null) {
      throw StateError('appRelease() requires a remote endpoint');
    }
    return releases.release(app, ttl: ttl);
  }

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

  // ==================== BACKENDS & BUCKETS ====================

  /// Enqueue a backend job (TS `run`).
  Future<void> run(
    String backend,
    String path,
    Map<String, dynamic> payload, {
    RunOptions? options,
  }) =>
      _dataModule.run(backend, path, payload, options: options);

  /// A handle to a storage bucket (TS `bucket`). Requires a remote endpoint.
  BucketHandle bucket(String name) {
    final remote = _remote;
    if (remote == null) throw StateError('bucket() requires a remote endpoint');
    return BucketHandle(name, remote);
  }

  // ==================== CRDT (deferred) ====================

  Future<Never> openCrdtField(String table, String recordId, String field,
          [String? fallbackText]) =>
      throw UnimplementedError('CRDT deferred');

  void closeCrdtField(String table, String recordId, String field) =>
      throw UnimplementedError('CRDT deferred');

  // ==================== LIFECYCLE ====================

  Future<void> close() async {
    _featureFlags?.closeAll();
    _appReleases?.closeAll();
    _dataModule.dispose();
    await _sync?.close();
    await _remote?.close();
    await _streamProcessor.close();
    _local.close();
    _initialized = false;
  }

  PersistenceClient _resolvePersistence() {
    final pc = config.persistenceClient;
    if (pc is PersistenceClient) return pc;
    if (pc == 'memory') return MemoryPersistenceClient();
    // Default to sqlite-backed persistence so stream-processor circuit state
    // and the auth token survive restarts (with a file-backed store).
    return SqlitePersistenceClient(_local);
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
