import { DataModule } from './modules/data/index';
import type {
  Sp00kyConfig,
  QueryTimeToLive,
  QueryStatusCallback,
  Sp00kyQueryResultPromise,
  PersistenceClient,
  PreloadOptions,
  UpdateOptions,
  RunOptions,
  SyncHealth} from './types';
import {
  LocalMigrator,
  RemoteDatabaseService,
  createLocalEngine,
} from './services/database/index';
import type { LocalStore } from './services/database/index';
import { StaleEpochError } from './services/database/index';
import type { UpEvent } from './modules/sync/index';
import { Sp00kySync } from './modules/sync/index';
import type {
  FinalQuery,
  GetTable,
  InnerQuery,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
  BucketNames,
  BackendNames,
  BackendRoutes,
  RoutePayload} from '@spooky-sync/query-builder';
import {
  QueryBuilder
} from '@spooky-sync/query-builder';

import { DevToolsService } from './modules/devtools/index';
import { createLogger } from './services/logger/index';
import { AuthService } from './modules/auth/index';
import { StreamProcessorService } from './services/stream-processor/index';
import { extractSelectPermissions } from './services/stream-processor/permissions';
import { EventSystem } from './events/index';
import { CacheModule } from './modules/cache/index';
import type { RecordWithId } from './modules/cache/index';
import { CrdtManager, CrdtField } from './modules/crdt/index';
import { preloadLoro } from './modules/crdt/loro-loader';
import { FeatureFlagModule, FeatureFlagHandle } from './modules/feature-flag/index';
import type { FeatureFlagOptions } from './modules/feature-flag/index';
import { LocalStoragePersistenceClient } from './services/persistence/localstorage';
import { ANON_USER_ID, bucketIdForUser } from './modules/ref-tables';
import { parseParams, encodeRecordId, parseDuration } from './utils/index';
import { SurrealDBPersistenceClient } from './services/persistence/surrealdb';
import { ResilientPersistenceClient } from './services/persistence/resilient';

export class BucketHandle {
  constructor(private bucketName: string, private remote: RemoteDatabaseService) {}

  async put(path: string, content: string | Uint8Array | Blob): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${path}".put($content);`, { content });
  }

  async get(path: string): Promise<unknown> {
    const [result] = await this.remote.query<[unknown]>(`RETURN f"${this.bucketName}:/${path}".get();`);
    return result;
  }

  async delete(path: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${path}".delete();`);
  }

  async exists(path: string): Promise<boolean> {
    const [result] = await this.remote.query<[boolean]>(`RETURN f"${this.bucketName}:/${path}".exists();`);
    return result;
  }

  async head(path: string): Promise<Record<string, unknown>> {
    const [result] = await this.remote.query<[Record<string, unknown>]>(`RETURN f"${this.bucketName}:/${path}".head();`);
    return result;
  }

  async copy(sourcePath: string, targetPath: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${sourcePath}".copy($target);`, { target: targetPath });
  }

  async rename(sourcePath: string, targetPath: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${sourcePath}".rename($target);`, { target: targetPath });
  }

  async list(prefix?: string): Promise<string[]> {
    const p = prefix ?? '';
    const [result] = await this.remote.query<[string[]]>(`RETURN f"${this.bucketName}:/${p}".list();`);
    return result;
  }
}

/**
 * Boot hint for which local bucket to open before auth resolves. Written to
 * PLAIN localStorage (never the configured persistenceClient): the surrealdb
 * persistence client stores its keys INSIDE a bucket, and the whole point of
 * the hint is to pick the bucket before any bucket is open. A warm reload of a
 * signed-in user thus opens their own bucket immediately — zero switches.
 * Losing the hint is fail-closed: boot lands on the anon bucket and the auth
 * callback switches to the user's bucket (cache + outbox intact).
 */
const LAST_BUCKET_KEY = 'sp00ky:last_bucket';

function readBootBucketHint(): string | null {
  try {
    return typeof localStorage !== 'undefined' ? localStorage.getItem(LAST_BUCKET_KEY) : null;
  } catch {
    return null;
  }
}

function writeBootBucketHint(bucketId: string): void {
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(LAST_BUCKET_KEY, bucketId);
  } catch {
    /* private-mode storage errors: boot just falls back to the anon bucket */
  }
}

export class Sp00kyClient<S extends SchemaStructure> {
  private local: LocalStore;
  private remote: RemoteDatabaseService;
  private persistenceClient: PersistenceClient;

  private migrator: LocalMigrator;
  private cache: CacheModule;
  private dataModule: DataModule<S>;
  private sync: Sp00kySync<S>;
  private devTools: DevToolsService;
  private crdtManager: CrdtManager;
  private featureFlags!: FeatureFlagModule<S>;
  // Query hashes already preloaded this session — skip redundant one-shot
  // fetches when the same preload query is requested again (e.g. a list row
  // re-rendering). Cleared on process/session end only.
  private preloadedHashes = new Set<number>();
  // In-flight background init chains (instant-hydrate + register enqueue) keyed
  // by registration hash. Concurrent mounts of the same query reuse the one
  // chain instead of double-hydrating and double-enqueuing `register`.
  // Sequential re-mounts intentionally start a fresh chain — the unconditional
  // `register` re-enqueue is what freshens a warm preload on use.
  private pendingQueryInits = new Map<string, Promise<void>>();

  private logger: ReturnType<typeof createLogger>;
  public auth: AuthService<S>;
  public streamProcessor: StreamProcessorService;

  get remoteClient() {
    return this.remote.getClient();
  }

  get localClient() {
    return this.local.getClient();
  }

  get pendingMutationCount(): number {
    return this.sync.pendingMutationCount;
  }

  /** Number of times the initial list_ref LIVE subscription retried on
   *  the most recent `setCurrentUserId` call. 0 when the SSP's
   *  pre-emptive user-table creation got there first; >0 when LIVE
   *  registration hit a "table not found" race. Exposed so the e2e
   *  suite can guard the pre-emptive path against regression. */
  get liveRetryCount(): number {
    return this.sync.liveRetryCount;
  }

  subscribeToPendingMutations(cb: (count: number) => void): () => void {
    return this.sync.subscribeToPendingMutations(cb);
  }

  /** Current sync-health snapshot. See {@link Sp00kyConfig.syncHealth}. */
  get syncHealth(): SyncHealth {
    return this.sync.syncHealth;
  }

  /**
   * Observe sync health. Fires immediately with the current status and again
   * on every healthy↔degraded transition. Returns an unsubscribe.
   */
  subscribeToSyncHealth(cb: (health: SyncHealth) => void): () => void {
    return this.sync.subscribeToSyncHealth(cb);
  }

  constructor(private config: Sp00kyConfig<S>) {
    const logger = createLogger(config.logLevel ?? 'info', config.otelTransmit);
    this.logger = logger.child({ service: 'Sp00kyClient' });

    this.logger.info(
      {
        config: { ...config, schema: '[SchemaStructure]' },
        Category: 'sp00ky-client::Sp00kyClient::constructor',
      },
      'Sp00kyClient initialized'
    );

    // Preload the loro CRDT engine at startup (fetches the chunk on page load)
    // so the first `openCrdtField` doesn't block on a network round-trip. Left
    // off, loro is never loaded unless a CRDT field is explicitly opened.
    if (config.crdt) void preloadLoro();

    // The default ('surrealdb') engine is a SurrealCacheEngine — a drop-in
    // subclass of LocalDatabaseService that adds the engine-neutral verb surface
    // with zero behavior change. Alternate engines (e.g. 'sqlite') require the
    // raw-SurrealQL call-site migration before they can back `this.local`.
    this.local = createLocalEngine(this.config.localEngine, this.config.database, logger);
    this.remote = new RemoteDatabaseService(this.config.database, logger);

    if (config.persistenceClient === 'surrealdb') {
      this.persistenceClient = new SurrealDBPersistenceClient(this.local, logger);
    } else if (config.persistenceClient === 'localstorage' || !config.persistenceClient) {
      this.persistenceClient = new LocalStoragePersistenceClient(logger);
    } else {
      this.persistenceClient = config.persistenceClient;
    }

    this.persistenceClient = new ResilientPersistenceClient(this.persistenceClient, logger);

    this.streamProcessor = new StreamProcessorService(
      new EventSystem(['stream_update']),
      this.local,
      this.persistenceClient,
      logger
    );
    this.migrator = new LocalMigrator(this.local, logger);

    this.cache = new CacheModule(
      this.local,
      this.streamProcessor,
      (update) => {
        // Direct callback from cache to data module
        this.dataModule.onStreamUpdate(update);
      },
      logger
    );

    // Initialize CRDT Manager. `local` is used to read the initial
    // `_00_crdt` snapshot when a field opens AND to mirror every local
    // edit (so reload/offline see the freshest state); `remote` is used
    // for the debounced outgoing UPSERTs and the parent-table LIVE feed.
    // The debounce window is configurable via `crdtDebounceMs`.
    this.crdtManager = new CrdtManager(
      this.config.schema,
      this.local,
      this.remote,
      logger,
      config.crdtDebounceMs ?? 500,
    );

    this.dataModule = new DataModule(
      this.cache,
      this.local,
      this.config.schema,
      logger,
      this.config.streamDebounceTime
    );

    // Initialize Auth
    this.auth = new AuthService(this.config.schema, this.remote, this.persistenceClient, logger);

    // Initialize Sync
    this.sync = new Sp00kySync(
      this.local,
      this.remote,
      this.cache,
      this.dataModule,
      this.config.schema,
      this.logger,
      {
        refSyncIntervalMs: this.config.refSyncIntervalMs,
        anonymousLiveQueries: this.config.enableAnonymousLiveQueries,
        // `syncHealth: false` (or `{ degradeAfterConsecutiveFailures: 0 }`)
        // disables degraded reporting; otherwise default to 3.
        degradeAfterConsecutiveFailures:
          this.config.syncHealth === false
            ? 0
            : this.config.syncHealth?.degradeAfterConsecutiveFailures ?? 3,
      }
    );

    // Initialize feature flags. Reuses the down-queue to register SSP plans
    // on `_00_user_feature` and the auth subscription to re-register handles
    // when the signed-in user changes.
    this.featureFlags = new FeatureFlagModule({
      dataModule: this.dataModule,
      sync: this.sync,
      auth: this.auth,
      logger,
    });

    // Initialize DevTools
    this.devTools = new DevToolsService(
      this.local,
      this.remote,
      logger,
      this.config.schema,
      this.auth,
      this.dataModule
    );

    // Register DevTools as a receiver for stream updates
    this.streamProcessor.addReceiver(this.devTools);

    // Wire up callbacks instead of events
    this.setupCallbacks();
  }

  /**
   * Setup direct callbacks instead of event subscriptions
   */
  private setupCallbacks() {
    // Surface query fetch-status changes (idle/fetching) in DevTools. Logs a
    // discrete event and triggers a state push so the active-queries panel
    // reflects the flip immediately.
    this.dataModule.onQueryStatusChange = (queryHash, status) => {
      this.devTools.logEvent('QUERY_STATUS_CHANGED', { queryHash, status });
    };

    // Keep an actively-watched query's remote `_00_query.lastActiveAt` fresh so
    // the server TTL sweep doesn't expire it out from under live subscribers.
    // DataModule fires this only while the query still has ≥1 subscriber.
    this.dataModule.onHeartbeat = (queryHash) => {
      void this.sync.heartbeatQuery(queryHash).catch((err) => {
        this.logger.warn(
          { err, queryHash, Category: 'sp00ky-client::Sp00kyClient::onHeartbeat' },
          'TTL heartbeat failed'
        );
      });
    };

    // Eager teardown of an opt-in deregistered query: enqueue a `cleanup`
    // down-event so it's serialized after any in-flight register/sync for the
    // same query (avoids out-of-order delete-before-create).
    this.dataModule.onDeregister = (queryHash) => {
      this.sync.enqueueDownEvent({ type: 'cleanup', payload: { hash: queryHash } });
    };

    // Mutation callback for sync
    this.dataModule.onMutation((mutations: UpEvent[]) => {
      // Notify DevTools
      this.devTools.onMutation(mutations);

      // Enqueue in Sync
      if (mutations.length > 0) {
        this.sync.enqueueMutation(mutations);
      }
    });

    // Sync events for incoming updates
    this.sync.events.subscribe('SYNC_QUERY_UPDATED', (event: any) => {
      this.devTools.logEvent('SYNC_QUERY_UPDATED', event.payload);
    });

    // Hand list_ref-driven row ingests to the CrdtManager so CRDT body
    // / cursor updates reach the receiver even when the cross-session
    // LIVE on the parent table is filtered out by the SurrealDB
    // permission-LIVE gap. Same-user clients receive these rows via
    // CrdtManager's own `LIVE SELECT * FROM <table>`; this hook is the
    // redundant path that fires when only the list_ref bumped.
    this.sync.engineEvents.subscribe('SYNC_REMOTE_DATA_INGESTED', (event: any) => {
      try {
        const records: Array<Record<string, any>> = event.payload?.records ?? [];
        for (const row of records) {
          const id = row?.id;
          const table =
            id && typeof id === 'object' && id.table !== undefined
              ? String(id.table)
              : undefined;
          if (!table) continue;
          this.crdtManager.applyRow(table, row);
        }
      } catch (err) {
        this.logger.debug(
          { err, Category: 'sp00ky-client::engineEvents::ingested' },
          'applyRow forwarding from sync ingest failed'
        );
      }
    });

    // Database events for DevTools
    this.local.getEvents().subscribe('DATABASE_LOCAL_QUERY', (event: any) => {
      this.devTools.logEvent('LOCAL_QUERY', event.payload);
    });

    this.remote.getEvents().subscribe('DATABASE_REMOTE_QUERY', (event: any) => {
      this.devTools.logEvent('REMOTE_QUERY', event.payload);
    });
  }

  async init() {
    this.logger.info(
      { Category: 'sp00ky-client::Sp00kyClient::init' },
      'Sp00kyClient initialization started'
    );
    try {
      // Open the bucket the last session used (per-user local stores). If auth
      // resolves to a different user below, the auth callback switches buckets.
      const bootBucket = readBootBucketHint() ?? ANON_USER_ID;
      await this.local.connect(bootBucket);
      this.logger.debug(
        { bootBucket, Category: 'sp00ky-client::Sp00kyClient::init' },
        'Local database connected'
      );

      // Schemaless local engines (SQLite) create tables lazily and need no
      // SurrealQL DDL provisioning.
      if (this.local.usesSurqlSchema) {
        await this.migrator.provision(this.config.schemaSurql);
        this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Schema provisioned');
      }

      await this.remote.connect();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'Remote database connected'
      );

      this.streamProcessor.setStateKeySuffix(bootBucket);
      await this.streamProcessor.init();
      // Seed table `select` permissions from the schema before any query is
      // registered — otherwise the SSP default-denies every non-`_00_` table.
      this.streamProcessor.setPermissions(
        extractSelectPermissions(this.config.schemaSurql)
      );
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'StreamProcessor initialized'
      );

      await this.auth.init();
      this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Auth initialized');

      // Salt query-id hashing with the SurrealDB session id so two browsers
      // for the same user don't collide on shared `_00_query` rows. The same
      // session id is the `session_id` key in `_00_cursor` rows, so the
      // CrdtManager needs it too.
      const sessionId = await this.fetchSessionId();
      await this.dataModule.init(sessionId);
      this.crdtManager.setSessionId(sessionId);
      this.logger.debug(
        { sessionId, Category: 'sp00ky-client::Sp00kyClient::init' },
        'DataModule initialized'
      );

      // Refresh the salt whenever auth state flips (sign-in, sign-out).
      // session::id() changes per WebSocket session, and a sign-in spawns
      // a new authenticated session, so the salt must follow. Also
      // forward the user id into `DataModule` and `Sp00kySync` so they
      // can route to per-user `_00_query_user_<id>` /
      // `_00_list_ref_user_<id>` tables in `RefMode.Dedicated` — the
      // LIVE subscription on `_00_list_ref_user_<id>` is restarted
      // under the new auth context inside `Sp00kySync.setCurrentUserId`
      // since SurrealDB binds the LIVE permission at registration time.
      //
      // Sync prefix BEFORE the first `await`: setting `currentUserId`
      // synchronously here is critical because the AuthProvider's own
      // subscribe callback runs right after ours and immediately enables
      // queries that depend on the user id. Any `await` before
      // `setCurrentUserId` would let those queries register against the
      // stale (null) user id and hit the wrong `_00_query[_user_*]`
      // table.
      this.auth.subscribe(async (userId) => {
        this.dataModule.setCurrentUserId(userId);
        // Mirror the server's `fn::query::register` auth injection for the
        // in-browser SSP: feed the current user's full record id + access
        // method so `$auth`-gated table permissions (e.g. `thread`) resolve
        // locally instead of being rejected. Set synchronously BEFORE the
        // first `await` (like `setCurrentUserId` above) so queries that
        // re-register on this auth flip see the fresh context, not a stale one.
        this.streamProcessor.setSessionAuth(
          this.auth.currentUser?.id ? encodeRecordId(this.auth.currentUser.id) : null,
          this.auth.access
        );
        // Record the target bucket synchronously (still before the first
        // `await`) so a reload mid-switch boots straight into the right store.
        writeBootBucketHint(bucketIdForUser(userId));
        // FIRST await: swap the local store to this user's bucket. Serialized
        // + latest-target-wins internally; no-op when the bucket already
        // matches (the boot-hint warm path).
        await this.ensureLocalBucket(userId);
        const next = await this.fetchSessionId();
        this.dataModule.setSessionId(next);
        this.crdtManager.setSessionId(next);
        try {
          await this.sync.setCurrentUserId(userId);
        } catch (e) {
          this.logger.error(
            { error: e, Category: 'sp00ky-client::Sp00kyClient::authChange' },
            'sync.setCurrentUserId failed'
          );
        }
      });

      await this.sync.init();
      this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Sync initialized');

      this.featureFlags.init();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'FeatureFlagModule initialized'
      );

      this.logger.info(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'Sp00kyClient initialization completed successfully'
      );
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::Sp00kyClient::init' },
        'Sp00kyClient initialization failed'
      );
      throw e;
    }
  }

  // Serializes bucket switches from rapid auth flips; `pendingBucketTarget`
  // makes intermediate targets collapse (A→anon→B never opens the anon bucket).
  private bucketSwitchChain: Promise<void> = Promise.resolve();
  private pendingBucketTarget: string | null = null;

  /**
   * Ensure the local store is this user's bucket, switching if needed. Called
   * from the auth listener on every auth flip; concurrent calls are chained
   * and superseded intermediates are skipped (latest target wins).
   */
  private ensureLocalBucket(userId: string | null): Promise<void> {
    const target = bucketIdForUser(userId);
    this.pendingBucketTarget = target;
    // Close the query gate SYNCHRONOUSLY the instant a switch is pending — the
    // AuthProvider's own auth subscriber fires right after this (same tick) and
    // enables queries, and `doSwitchBucket` only runs a microtask later on the
    // chain. Without closing the gate here, that query is issued through the
    // still-open gate and is in-flight on the local wasm engine when
    // `switchStore` closes the client — which wedges the engine (every
    // subsequent query, including provisioning, hangs → no view ever registers).
    // No-op when already on the target bucket.
    const needsSwitch = this.local.currentBucketId !== target;
    const release = needsSwitch ? this.local.beginSwitch() : null;
    this.bucketSwitchChain = this.bucketSwitchChain.then(async () => {
      // Superseded by a newer flip, or already on target: reopen the gate we
      // closed above and skip the switch.
      if (this.pendingBucketTarget !== target || this.local.currentBucketId === target) {
        release?.();
        return;
      }
      await this.doSwitchBucket(target, release);
    });
    // Isolate chain failures per-caller: a failed switch must not poison every
    // future switch. The caller (auth listener) logs it. Reopen the gate on
    // failure so the client never gets stuck closed.
    const result = this.bucketSwitchChain;
    this.bucketSwitchChain = this.bucketSwitchChain.catch(() => {
      release?.();
    });
    return result;
  }

  /**
   * The bucket-switch choreography: drain → swap → rebind.
   *
   * Drain: sync quiesced (poll/LIVE stopped, in-flight round awaited so its
   * outbox delete lands in the OLD bucket, debounce timers cancelled),
   * DataModule timers cleared, CRDT fields closed WITHOUT their final flush
   * (the remote session already belongs to the next user).
   *
   * Swap: gate closes so any local query issued mid-switch (sibling auth
   * subscribers, FeatureFlagModule) waits and then runs against the NEW
   * bucket; store swaps open-new-before-close-old; schema provisions
   * (no-op for a returning bucket); stale `_00_query` rows are wiped (dead
   * sessionId-salted hashes with stale arrays — record bodies stay warm);
   * SSP resets to a fresh circuit with re-seeded permissions.
   *
   * Rebind: auth token re-persisted (the surrealdb persistence client wrote it
   * into the OLD bucket's `_00_kv` before this listener ran), active queries
   * re-homed keeping their hashes, sync resumed on the new bucket's own
   * outbox, and every query re-registered remotely to refill from the server.
   */
  private async doSwitchBucket(target: string, gateRelease?: (() => void) | null): Promise<void> {
    this.logger.info(
      { target, from: this.local.currentBucketId, Category: 'sp00ky-client::Sp00kyClient::doSwitchBucket' },
      'Switching local bucket'
    );

    await this.sync.prepareBucketSwitch();
    this.dataModule.quiesce();
    this.crdtManager.closeAll({ flush: false });

    // Reuse the gate the caller (`ensureLocalBucket`) closed synchronously; only
    // open our own if called without one (keeps the gate continuously closed
    // from the auth flip through the swap — no window for a racing query).
    const reopen = gateRelease ?? this.local.beginSwitch();
    try {
      await this.local.switchStore(target);
      if (this.local.usesSurqlSchema) {
        await this.migrator.provision(this.config.schemaSurql);
      }
      await this.local.queryUngated('DELETE _00_query;');
      this.streamProcessor.setStateKeySuffix(target);
      await this.streamProcessor.reset();
      this.streamProcessor.setPermissions(extractSelectPermissions(this.config.schemaSurql));
      this.cache.clearVersionLookups();
      // Preload dedup is per-bucket: the `_00_preload` markers + cached rows it
      // guards live in the local store we just swapped away from. Keeping the
      // hashes would make `preload()` skip warming the NEW bucket (its store is
      // empty), so every thread/comment prewarm silently no-ops after login.
      this.preloadedHashes.clear();
    } finally {
      reopen();
    }

    if (this.auth.token) {
      try {
        await this.persistenceClient.set('sp00ky_auth_token', this.auth.token);
      } catch (e) {
        this.logger.warn(
          { error: e, Category: 'sp00ky-client::Sp00kyClient::doSwitchBucket' },
          'Failed to re-persist auth token into the new bucket'
        );
      }
    }

    const hashes = await this.dataModule.rebindAfterBucketSwitch();
    await this.sync.completeBucketSwitch();
    for (const hash of hashes) {
      this.sync.enqueueDownEvent({ type: 'register', payload: { hash } });
    }

    this.logger.info(
      { target, queries: hashes.length, Category: 'sp00ky-client::Sp00kyClient::doSwitchBucket' },
      'Local bucket switch complete'
    );
  }

  async close() {
    await this.featureFlags.closeAll();
    this.crdtManager.closeAll();
    await this.local.close();
    await this.remote.close();
  }

  /**
   * Subscribe to a feature flag for the current user. Returns a
   * `FeatureFlagHandle` whose `variant()`, `payload()` and `enabled()`
   * accessors reflect the latest assignment from `_00_user_feature`,
   * and whose `subscribe(cb)` fires whenever that assignment changes.
   *
   * Permissions are enforced by SurrealDB: a client can only ever see
   * its own row, and cannot create or modify assignments.
   */
  feature(key: string, options?: FeatureFlagOptions): FeatureFlagHandle {
    return this.featureFlags.feature(key, options);
  }

  authenticate(token: string) {
    return this.remote.getClient().authenticate(token);
  }

  /**
   * Open a CRDT field for collaborative editing.
   * Returns a CrdtField with a LoroDoc that can be bound to any editor.
   * Also starts a LIVE SELECT on the parent table for real-time sync;
   * incoming events trigger a subquery fetch of `_00_crdt` / `_00_cursor`.
   */
  async openCrdtField(
    table: string,
    recordId: string,
    field: string,
    fallbackText?: string,
  ): Promise<CrdtField> {
    return this.crdtManager.open(table, recordId, field, fallbackText);
  }

  /**
   * Close a CRDT field when editing is done.
   */
  closeCrdtField(table: string, recordId: string, field: string): void {
    this.crdtManager.close(table, recordId, field);
  }

  deauthenticate() {
    return this.remote.getClient().invalidate();
  }

  query<Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>,
    ttl: QueryTimeToLive = '10m'
  ): QueryBuilder<S, Table, Sp00kyQueryResultPromise> {
    return new QueryBuilder<S, Table, Sp00kyQueryResultPromise>(
      this.config.schema,
      table,
      async (q) => ({
        hash: await this.initQuery(table, q, ttl),
      }),
      options
    );
  }

  private async initQuery<Table extends TableNames<S>>(
    table: Table,
    q: InnerQuery<any, any, any>,
    ttl: QueryTimeToLive
  ) {
    const tableSchema = this.config.schema.tables.find((t) => t.name === table);
    if (!tableSchema) {
      throw new Error(`Table ${table} not found`);
    }

    const params = parseParams(tableSchema.columns, q.selectQuery.vars ?? {});
    const hash = await this.dataModule.query(
      table,
      q.selectQuery.query,
      params,
      ttl,
      q.selectQuery.plan
    );

    // Local-first paint: the hash is returned as soon as the LOCAL registration
    // above completes — `queryState.records` is already seeded from the local
    // cache/SSP snapshot, so `useQuery` subscribes and paints from memory with
    // zero network on the paint path. Instant-hydrate and the `register`
    // down-event continue in a background chain (hydrate strictly before
    // enqueue, so a stale one-shot snapshot can never land after the sync's
    // authoritative `_00_list_ref` overwrite). Concurrent mounts of the same
    // query share one chain; a sequential re-mount starts a fresh one so its
    // `register` re-enqueue keeps freshening warm data on use.
    if (!this.pendingQueryInits.has(hash)) {
      const chain = this.finishQueryInit(hash, q, params).finally(() => {
        this.pendingQueryInits.delete(hash);
      });
      this.pendingQueryInits.set(hash, chain);
    }

    return hash;
  }

  /**
   * Background tail of {@link initQuery}: instant-hydrate (opt-in via
   * `config.instantHydrate`, and only when the query is cold) followed by
   * enqueuing the `register` down-event. Never rejects — both halves catch and
   * log, so `void`-ing the returned promise can't produce an unhandled
   * rejection. By default (hydrate off) the register lifecycle is the single
   * freshness path; the one-shot fetch is an optimization apps enable
   * explicitly, and it runs regardless of preload state — cache-first delivery
   * never depends on WHY rows are cached.
   */
  private async finishQueryInit(
    hash: string,
    q: InnerQuery<any, any, any>,
    params: Record<string, any>
  ): Promise<void> {
    if (this.config.instantHydrate === true && this.dataModule.isCold(hash)) {
      try {
        // Fence against bucket switches: rows fetched under the previous
        // auth context must not hydrate the new bucket's query state — the
        // rebind's re-registration refills it from the right context.
        const epoch = this.local.epoch;
        const [rows] = await this.remote.query<[RecordWithId[]]>(q.selectQuery.query, params);
        if (epoch === this.local.epoch) {
          await this.dataModule.applyHydration(hash, rows ?? []);
        }
      } catch (err) {
        if (err instanceof StaleEpochError) {
          this.logger.debug(
            { hash, Category: 'sp00ky-client::Sp00kyClient::instantHydrate' },
            'Dropped instant hydrate from before a bucket switch'
          );
        } else {
          this.logger.warn(
            { err, hash, Category: 'sp00ky-client::Sp00kyClient::instantHydrate' },
            'Instant hydrate failed; proceeding with registration'
          );
        }
      }
    }

    try {
      await this.sync.enqueueDownEvent({
        type: 'register',
        payload: {
          hash,
        },
      });
    } catch (err) {
      this.logger.error(
        { err, hash, Category: 'sp00ky-client::Sp00kyClient::initQuery' },
        'Failed to enqueue register down-event'
      );
    }
  }

  /**
   * Smart, awaitable preload/prewarm into the LOCAL cache — without registering a
   * live view (NO `_00_query`, NO subscription, NO TTL heartbeat).
   *
   * Cache-aware via a durable per-bucket freshness marker (`_00_preload`):
   * - COLD (never preloaded in this bucket): fetch the query one-shot from the
   *   remote, persist the rows (+ embedded `.related()` children), stamp the
   *   marker — and AWAIT it. This is the "smart waiting" first load: callers can
   *   `await db.preload(...)` to hold the UI until the data is ready.
   * - WARM (marker present): return instantly — NEVER blocks. `refresh` decides
   *   whether to also kick a one-time silent refetch (see {@link PreloadOptions}).
   *   Default `onUse` does nothing; the data freshens when the real `useQuery`
   *   mounts and registers its live view.
   *
   * Best-effort: any fetch failure (offline, etc.) is a no-op warn (no marker
   * written, so it's retried next load). Deduped per session by query hash.
   */
  async preload(
    finalQuery: FinalQuery<S, any, any, any, any, any>,
    options?: PreloadOptions
  ): Promise<void> {
    const q = finalQuery.innerQuery;
    if (this.preloadedHashes.has(q.hash)) return;

    const tableName = q.tableName;
    const tableSchema = this.config.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }
    const params = parseParams(tableSchema.columns, q.selectQuery.vars ?? {});
    const hashKey = String(q.hash);

    const marker = await this.dataModule.getPreloadMarker(hashKey);

    // COLD → fetch + persist + stamp, awaited so the caller can block on it.
    if (!marker) {
      const rowCount = await this.fetchAndPersist(q, tableName, params);
      if (rowCount >= 0) {
        await this.dataModule.writePreloadMarker(hashKey, rowCount);
        this.preloadedHashes.add(q.hash);
      }
      return;
    }

    // WARM → never block. Mark handled for this session, then optionally refresh.
    this.preloadedHashes.add(q.hash);
    const refresh = options?.refresh ?? 'onUse';
    if (refresh === 'onUse') return;

    if (refresh === 'stale') {
      const maxAgeMs = parseDuration(options?.staleTime ?? '1h');
      if (Date.now() - marker.fetchedAt <= maxAgeMs) return; // still fresh
    }

    // `background`, or `stale` past its staleTime → one-time silent refetch.
    void this.fetchAndPersist(q, tableName, params).then((rowCount) => {
      if (rowCount >= 0) return this.dataModule.writePreloadMarker(hashKey, rowCount);
    });
  }

  /**
   * One-shot remote fetch + local persist for a preload query. Returns the row
   * count on success, or -1 on failure (best-effort: logged, never thrown) so
   * the caller skips stamping the freshness marker and retries next load.
   */
  private async fetchAndPersist(
    q: InnerQuery<any, any, any>,
    tableName: string,
    params: Record<string, any>
  ): Promise<number> {
    try {
      const [rows] = await this.remote.query<[RecordWithId[]]>(q.selectQuery.query, params);
      const list = rows ?? [];
      await this.dataModule.persistSnapshot(tableName, list);
      return list.length;
    } catch (err) {
      this.logger.warn(
        { err, hash: q.hash, Category: 'sp00ky-client::Sp00kyClient::preload' },
        'Preload fetch failed; data will be fetched on demand'
      );
      return -1;
    }
  }

  async queryRaw(sql: string, params: Record<string, any>, ttl: QueryTimeToLive) {
    const tableName = sql.split('FROM ')[1].split(' ')[0];
    return this.dataModule.query(tableName, sql, params, ttl);
  }

  async subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options?: { immediate?: boolean }
  ): Promise<() => void> {
    return this.dataModule.subscribe(queryHash, callback, options);
  }

  /**
   * Opt-in eager teardown for a query whose last subscriber has gone away
   * (e.g. a viewport-windowed list cancelling an off-screen window). No-op
   * while any subscriber remains. Tears down the remote `_00_query` view +
   * local WASM view instead of waiting for the TTL sweep. Default behavior
   * (no call here) keeps the view resident for cheap re-subscription.
   */
  deregisterQuery(queryHash: string): void {
    this.dataModule.deregisterQuery(queryHash);
  }

  /**
   * Subscribe to a query's fetch-status changes (idle/fetching). With
   * `{ immediate: true }` the callback fires synchronously with the current
   * status. Powers the `useQuery` hook's `isFetching()` accessor.
   */
  subscribeQueryStatus(
    queryHash: string,
    callback: QueryStatusCallback,
    options?: { immediate?: boolean }
  ): () => void {
    return this.dataModule.subscribeStatus(queryHash, callback, options);
  }

  /**
   * Report the frontend processing time (ms) a client framework spent applying
   * an update for a query (e.g. `useQuery`'s `reconcile()`), so DevTools/MCP can
   * surface the "frontend" phase of the per-query timing breakdown.
   */
  reportFrontendTiming(queryHash: string, ms: number): void {
    this.dataModule.recordFrontendTiming(queryHash, ms);
  }

  run<
    B extends BackendNames<S>,
    R extends BackendRoutes<S, B>,
  >(backend: B, path: R, payload: RoutePayload<S, B, R>, options?: RunOptions) {
    return this.dataModule.run(backend, path, payload, options);
  }

  runRecurring<
    B extends BackendNames<S>,
    R extends BackendRoutes<S, B>,
  >(
    backend: B,
    path: R,
    payload: RoutePayload<S, B, R>,
    options: RunOptions & { interval: number; assignedTo: string }
  ) {
    return this.dataModule.runRecurring(backend, path, payload, options);
  }

  pokeRecurring<B extends BackendNames<S>>(
    backend: B,
    path: BackendRoutes<S, B>,
    options: { assignedTo: string }
  ) {
    return this.dataModule.pokeRecurring(backend, path, options);
  }

  cancelRecurring<B extends BackendNames<S>>(
    backend: B,
    path: BackendRoutes<S, B>,
    options: { assignedTo: string }
  ) {
    return this.dataModule.cancelRecurring(backend, path, options);
  }

  bucket<B extends BucketNames<S>>(name: B): BucketHandle {
    return new BucketHandle(name, this.remote);
  }

  create(id: string, data: Record<string, unknown>) {
    return this.dataModule.create(id, data);
  }

  update(table: string, id: string, data: Record<string, unknown>, options?: UpdateOptions) {
    return this.dataModule.update(table, id, data, options);
  }

  delete(table: string, id: string) {
    return this.dataModule.delete(table, id);
  }

  async useRemote<T>(fn: (client: Surreal) => Promise<T> | T): Promise<T> {
    return fn(this.remote.getClient());
  }

  /**
   * Fetch SurrealDB's `session::id()` as a string. Used as a salt for
   * query-id hashing so two sessions for the same user get distinct
   * `_00_query` rows. Returns empty string if the query fails (we still
   * boot, just without session scoping for IDs).
   */
  private async fetchSessionId(): Promise<string> {
    try {
      const [sid] = await this.remote.query<[string]>('RETURN <string>session::id()');
      return typeof sid === 'string' ? sid : '';
    } catch (e) {
      this.logger.warn(
        { error: e, Category: 'sp00ky-client::Sp00kyClient::fetchSessionId' },
        'Failed to fetch session::id() — proceeding with empty salt'
      );
      return '';
    }
  }
}
