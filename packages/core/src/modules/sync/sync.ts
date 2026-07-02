import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { RecordVersionArray, RecordVersionDiff, SyncHealth, SyncHealthStatus } from '../../types';
import { createSyncEventSystem, SyncEventTypes, SyncQueueEventTypes } from './events/index';
import type { Logger } from '../../services/logger/index';
import type { DownEvent, UpEvent} from './queue/index';
import { DownQueue, UpQueue } from './queue/index';
import type { RecordId, Uuid } from 'surrealdb';
import {
  ArraySyncer,
  buildListRefSelect,
  buildSubqueryListRefSelect,
  createDiffFromDbOp,
  diffRecordVersionArray,
  listRefPollDelayMs,
  recordVersionArraysEqual,
  resolveListRefPollInterval,
} from './utils';
import { SyncEngine } from './engine';
import { SyncScheduler } from './scheduler';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { CacheModule } from '../cache/index';
import type { DataModule } from '../data/index';
import { classifySyncError, encodeRecordId, extractIdPart, extractTablePart, surql } from '../../utils/index';
import { ANON_USER_ID, DEFAULT_REF_MODE, listRefTableFor, RefMode } from '../ref-tables';

/**
 * Tunables for `Sp00kySync` construction.
 */
export interface Sp00kySyncOptions {
  /**
   * Cadence (ms) for the `_00_list_ref` poll fallback that catches
   * cross-session UPDATEs the LIVE-permission gap drops. Non-positive
   * values fall back to the default; see
   * {@link resolveListRefPollInterval}.
   */
  refSyncIntervalMs?: number;
  /**
   * Enable realtime sync for unauthenticated clients against the shared
   * `_00_list_ref_anon` table. See {@link Sp00kyConfig.enableAnonymousLiveQueries}.
   * Defaults to `false`.
   */
  anonymousLiveQueries?: boolean;
  /**
   * Consecutive failed sync rounds before sync health flips to `degraded`.
   * `0` disables degraded reporting. See {@link Sp00kyConfig.syncHealth}.
   * Defaults to `3`.
   */
  degradeAfterConsecutiveFailures?: number;
}

/**
 * The main synchronization engine for Sp00ky.
 * Handles the bidirectional synchronization between the local database and the remote backend.
 * Uses a queue-based architecture with 'up' (local to remote) and 'down' (remote to local) queues.
 * @template S The schema structure type.
 */
export class Sp00kySync<S extends SchemaStructure> {
  private upQueue: UpQueue;
  private downQueue: DownQueue;
  private isInit: boolean = false;
  private logger: Logger;
  private syncEngine: SyncEngine;
  /** Engine-level events (e.g. `SYNC_REMOTE_DATA_INGESTED`). Distinct
   *  from `this.events`, which carries Sp00kySync-level events like
   *  `SYNC_QUERY_UPDATED` and `SYNC_MUTATION_ROLLED_BACK`. */
  public get engineEvents() {
    return this.syncEngine.events;
  }
  private scheduler: SyncScheduler;
  private wasDisconnected: boolean = false;
  public events = createSyncEventSystem();

  // Auth identity that drives per-user `_00_list_ref_user_<id>` routing
  // in `RefMode.Dedicated`. Updated by `setCurrentUserId` from the auth
  // subscription in `Sp00kyClient`; null when unauthenticated.
  private currentUserId: string | null = null;

  private refMode: RefMode = DEFAULT_REF_MODE;

  // When true, an unauthenticated client still runs the `_00_list_ref` poll
  // and LIVE subscription, routed to the shared `_00_list_ref_anon` table, so
  // a logged-out page gets realtime `useQuery` updates. Off by default.
  private readonly anonLiveEnabled: boolean;

  // Bookkeeping for the LIVE subscription on `_00_list_ref[_user_*]`.
  // SurrealDB binds the permission context at LIVE-registration time and
  // the table name in dedicated mode depends on the authenticated user,
  // so we have to re-register whenever auth state flips.
  private currentLiveQueryUuid: Uuid | null = null;
  private liveQueryUnsubscribe: (() => void) | null = null;

  // Periodic re-poll of `_00_list_ref` as a safety net for missed LIVE
  // notifications. SurrealDB v3 occasionally drops LIVE deliveries
  // across sessions even when the row matches the permission rule;
  // this catches those without requiring users to reload. The
  // interval is configurable via the constructor; see
  // `resolveListRefPollInterval` for fallback semantics.
  //
  // Self-rescheduling rather than setInterval so each tick can pick
  // its own delay via `nextPollDelayMs` — slows the poll down when
  // LIVE is delivering events and speeds it back up when LIVE quiets.
  private listRefPollTimer: ReturnType<typeof setTimeout> | null = null;
  private listRefPollRunning: boolean = false;
  // The currently-executing poll tick, if any. `stopListRefPoll` only stops
  // future ticks; a bucket switch must also AWAIT the in-flight one so its
  // local writes land in the store it started against.
  private listRefPollInFlight: Promise<void> | null = null;
  public readonly refSyncIntervalMs: number;

  // Consecutive poll cycles that observed NO list_ref change. Drives the
  // adaptive backoff in `startListRefPoll` via `listRefPollDelayMs`: an idle
  // page coasts from the fast base cadence toward the 5s cap, and any activity
  // (a poll-detected change or a LIVE event) resets it to 0 so the poll snaps
  // back to responsive. Replaces the old LIVE-liveness backoff, which kept the
  // poll pinned at 500ms forever whenever LIVE wasn't firing (the common case
  // on a quiet page, thanks to the cross-session LIVE-permission gap).
  private listRefIdleStreak: number = 0;

  // `${queryHash}:${recordId}` -> consecutive rounds the id has been "still
  // remote" (left the query's list_ref but still exists upstream). Used to
  // distinguish a PERSISTENT view-membership disagreement (the `job:` churn,
  // converged once it crosses the threshold) from a record that's merely
  // mid-deletion (still-remote for ~one round, then gone) — which must NOT be
  // converged, or it gets stranded in this window before its delete is observed.
  private stillRemoteStreaks: Map<string, number> = new Map();

  // Wall-clock timestamp (ms) of the most recent LIVE event delivered
  // through `handleRemoteListRefChange`. Kept as a diagnostic / liveness
  // signal; the poll cadence is now driven by `listRefIdleStreak`.
  private lastLiveEventAt: number | null = null;

  // Number of times the initial `_00_list_ref[_user_*]` LIVE subscription
  // had to retry on `setCurrentUserId`. Stays at 0 when the SSP has
  // pre-emptively created the user's dedicated tables; otherwise
  // increments on each retry attempt until LIVE succeeds or attempts
  // are exhausted. Surfaced as a diagnostic so the e2e suite can prove
  // the pre-emptive table-creation path is keeping the first sign-in
  // off the lazy-creation race.
  private _liveRetryCount: number = 0;
  public get liveRetryCount(): number {
    return this._liveRetryCount;
  }

  get isSyncing() {
    return this.scheduler.isSyncing;
  }

  get pendingMutationCount(): number {
    return this.upQueue.size;
  }

  subscribeToPendingMutations(cb: (count: number) => void): () => void {
    const id1 = this.upQueue.events.subscribe(
      SyncQueueEventTypes.MutationEnqueued,
      (event) => cb(event.payload.queueSize)
    );
    const id2 = this.upQueue.events.subscribe(
      SyncQueueEventTypes.MutationDequeued,
      (event) => cb(event.payload.queueSize)
    );
    return () => {
      this.upQueue.events.unsubscribe(id1);
      this.upQueue.events.unsubscribe(id2);
    };
  }

  // ---- Sync health -------------------------------------------------------
  // `0` disables degraded reporting (config `syncHealth: false`). Resolved
  // from config in Sp00kyClient and passed through the constructor options.
  private readonly degradeAfterFailures: number;
  private consecutiveSyncFailures = 0;
  private syncHealthStatus: SyncHealthStatus = 'healthy';
  private lastSyncErrorKind: 'network' | 'application' | undefined;
  private lastSyncErrorMessage: string | undefined;
  // Latched `true` on the first successful sync round; never reset. Lets a UI
  // tell a cold-start "connecting" phase (never reached the server) apart from
  // a real lost connection after a working session.
  private hasSyncedOnce = false;

  // Self-heal: while degraded, re-drive sync on an exponential backoff so the
  // app recovers on its own — even when the socket never actually dropped (in
  // that case no `connected` event fires, so this re-registration is the ONLY
  // thing that re-probes the server). Started on the degrade transition,
  // cleared on recovery. Capped cadence so a long outage doesn't busy-loop.
  private selfHealTimer: ReturnType<typeof setTimeout> | null = null;
  private selfHealAttempts = 0;
  private static readonly SELF_HEAL_BASE_MS = 2_000;
  private static readonly SELF_HEAL_MAX_MS = 30_000;

  /** Current sync-health snapshot. */
  get syncHealth(): SyncHealth {
    return {
      status: this.syncHealthStatus,
      consecutiveFailures: this.consecutiveSyncFailures,
      kind: this.syncHealthStatus === 'degraded' ? this.lastSyncErrorKind : undefined,
      error: this.syncHealthStatus === 'degraded' ? this.lastSyncErrorMessage : undefined,
      everConnected: this.hasSyncedOnce,
    };
  }

  /**
   * Observe sync health. The callback fires immediately with the current
   * status and again on every healthy↔degraded transition. Returns an
   * unsubscribe. Mirrors {@link subscribeToPendingMutations}.
   */
  subscribeToSyncHealth(cb: (health: SyncHealth) => void): () => void {
    cb(this.syncHealth);
    const id = this.events.subscribe(SyncEventTypes.SyncHealthChanged, (event) =>
      cb(event.payload)
    );
    return () => this.events.unsubscribe(id);
  }

  private emitSyncHealth(): void {
    this.events.emit(SyncEventTypes.SyncHealthChanged, this.syncHealth);
  }

  /**
   * Fed by the scheduler once per drained sync round. Individual failures are
   * absorbed by the queue's retry; only a run of `degradeAfterFailures`
   * consecutive failures flips the status to `degraded`, and the next clean
   * round flips it back. No-op when reporting is disabled (`degradeAfterFailures`
   * is 0).
   */
  private recordSyncOutcome(ok: boolean, error?: unknown): void {
    if (this.degradeAfterFailures <= 0) return;
    if (ok) {
      // Latch first-ever success so a UI can drop the connecting phase. Set
      // before the early return so a clean cold start (0 prior failures) counts.
      this.hasSyncedOnce = true;
      if (this.consecutiveSyncFailures === 0) return;
      this.consecutiveSyncFailures = 0;
      if (this.syncHealthStatus !== 'healthy') {
        this.syncHealthStatus = 'healthy';
        this.lastSyncErrorKind = undefined;
        this.lastSyncErrorMessage = undefined;
        this.stopSelfHeal();
        this.logger.info(
          { Category: 'sp00ky-client::Sp00kySync::syncHealth' },
          'Sync recovered; health back to healthy'
        );
        this.emitSyncHealth();
      }
      return;
    }
    this.consecutiveSyncFailures++;
    this.lastSyncErrorKind = classifySyncError(error);
    this.lastSyncErrorMessage = error instanceof Error ? error.message : String(error);
    if (
      this.syncHealthStatus !== 'degraded' &&
      this.consecutiveSyncFailures >= this.degradeAfterFailures
    ) {
      this.syncHealthStatus = 'degraded';
      this.logger.warn(
        {
          consecutiveFailures: this.consecutiveSyncFailures,
          kind: this.lastSyncErrorKind,
          error,
          Category: 'sp00ky-client::Sp00kySync::syncHealth',
        },
        'Sync degraded after sustained failures'
      );
      this.emitSyncHealth();
      this.startSelfHeal();
    }
  }

  /**
   * Begin self-heal retries (no-op if already running). Started on the
   * healthy→degraded transition; {@link recordSyncOutcome} stops it on recovery.
   */
  private startSelfHeal(): void {
    if (this.selfHealTimer !== null) return;
    this.selfHealAttempts = 0;
    this.scheduleSelfHeal();
  }

  private scheduleSelfHeal(): void {
    const delay = Math.min(
      Sp00kySync.SELF_HEAL_MAX_MS,
      Sp00kySync.SELF_HEAL_BASE_MS * 2 ** this.selfHealAttempts
    );
    this.selfHealTimer = setTimeout(async () => {
      this.selfHealTimer = null;
      if (this.syncHealthStatus !== 'degraded') return;
      this.selfHealAttempts++;
      this.logger.debug(
        { attempt: this.selfHealAttempts, delayMs: delay, Category: 'sp00ky-client::Sp00kySync::selfHeal' },
        'Self-heal: re-driving sync while degraded'
      );
      try {
        // Retry whatever is still queued first; the failing op (register or
        // mutation) was re-queued by the queue, so this re-probes the server
        // and reports the outcome through the scheduler → recordSyncOutcome.
        if (this.upQueue.size > 0) {
          await this.scheduler.syncUp();
        } else if (this.downQueue.size > 0) {
          await this.scheduler.syncDown();
        } else {
          // Nothing queued (e.g. the failing op was rolled back + dropped):
          // re-register active queries — mirroring the reconnect handler — so
          // there's a concrete op whose success flips health. If there are no
          // active queries either, probe connectivity directly.
          const hashes = this.dataModule.getActiveQueryHashes();
          if (hashes.length > 0) {
            for (const hash of hashes) {
              this.scheduler.enqueueDownEvent({ type: 'register', payload: { hash } });
            }
            await this.scheduler.syncDown();
          } else {
            await this.remote.query('RETURN true');
            this.recordSyncOutcome(true);
          }
        }
      } catch (err) {
        // Only the direct connectivity probe can throw here (syncUp/syncDown
        // swallow + self-report); treat a probe failure as another failed round.
        this.recordSyncOutcome(false, err);
      }
      // Keep retrying until recovery. recordSyncOutcome(true) calls stopSelfHeal
      // (clearing any pending timer), so only continue while still degraded.
      if (this.syncHealthStatus === 'degraded') this.scheduleSelfHeal();
    }, delay);
  }

  private stopSelfHeal(): void {
    if (this.selfHealTimer !== null) {
      clearTimeout(this.selfHealTimer);
      this.selfHealTimer = null;
    }
    this.selfHealAttempts = 0;
  }

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private cache: CacheModule,
    private dataModule: DataModule<S>,
    private schema: S,
    logger: Logger,
    options?: Sp00kySyncOptions
  ) {
    this.logger = logger.child({ service: 'Sp00kySync' });
    this.upQueue = new UpQueue(this.local, this.logger);
    this.downQueue = new DownQueue(this.local, this.logger);
    this.syncEngine = new SyncEngine(this.remote, this.cache, this.schema, this.logger);
    this.scheduler = new SyncScheduler(
      this.upQueue,
      this.downQueue,
      this.processUpEvent.bind(this),
      this.processDownEvent.bind(this),
      this.logger,
      this.handleRollback.bind(this),
      this.recordSyncOutcome.bind(this)
    );
    this.refSyncIntervalMs = resolveListRefPollInterval(options?.refSyncIntervalMs);
    this.anonLiveEnabled = options?.anonymousLiveQueries ?? false;
    this.degradeAfterFailures = Math.max(0, options?.degradeAfterConsecutiveFailures ?? 3);
  }

  /**
   * Initializes the synchronization system.
   * Starts the scheduler and initiates the initial sync cycles.
   * @throws Error if already initialized.
   */
  public async init() {
    if (this.isInit) throw new Error('Sp00kySync is already initialized');
    this.isInit = true;
    await this.scheduler.init();
    this.subscribeToReconnect();
    void this.scheduler.syncUp();
    void this.scheduler.syncDown();
    // No initial LIVE subscription — wait for `setCurrentUserId` to fire
    // from the auth subscription. In dedicated mode the table name
    // depends on the authenticated user, and an unauthenticated
    // subscription wouldn't match any of the per-user tables anyway.
    //
    // Exception: when anonymous live queries are enabled, start realtime now
    // against the shared `_00_list_ref_anon` table so a logged-out client
    // syncs immediately. `setCurrentUserId` re-points LIVE to the per-user
    // table on sign-in. Guard on `currentUserId` because the auth callback can
    // fire (and authenticate) before `init()` runs — don't clobber that back
    // to the anon table. `setCurrentUserId(null)` is a no-op on first load
    // (it's already null), so this is the only place anon realtime starts.
    if (this.anonLiveEnabled && !this.currentUserId) {
      this.startListRefPoll();
      this.restartRefLiveQuery().catch((err) => {
        this.logger.debug(
          { err, Category: 'sp00ky-client::Sp00kySync::init' },
          'Anonymous ref LIVE start failed; relying on periodic poll fallback'
        );
      });
    }
  }

  /**
   * Quiesce all sync activity ahead of a local-bucket switch. After this
   * resolves, nothing in the sync module writes to the local store: the poll
   * loop is stopped AND its in-flight tick awaited, LIVE is killed, debounce
   * timers are cancelled (their outbox rows are already persisted), and the
   * scheduler has drained its in-flight queue item — including that item's
   * outbox-row delete, which must land in the OLD bucket. Queued down-events
   * are dropped (they reference old-bucket query rows; the post-switch rebind
   * re-enqueues registrations). The old user's un-pushed outbox is deliberately
   * NOT drained: the remote session already belongs to the next user.
   */
  public async prepareBucketSwitch(): Promise<void> {
    this.stopSelfHeal();
    this.stopListRefPoll();
    if (this.listRefPollInFlight) await this.listRefPollInFlight;
    await this.killRefLiveQuery();
    this.upQueue.clearDebounceTimers();
    await this.scheduler.pause();
    this.downQueue.clear();
    this.stillRemoteStreaks.clear();
    this.logger.info(
      { Category: 'sp00ky-client::Sp00kySync::prepareBucketSwitch' },
      'Sync quiesced for bucket switch'
    );
  }

  /**
   * Resume syncing against the freshly-opened bucket: reload the mutation
   * outbox from ITS `_00_pending_mutations` (the new user's own un-pushed
   * offline work) and restart the scheduler. LIVE + the list_ref poll restart
   * via the `setCurrentUserId` call that follows in the auth listener.
   */
  public async completeBucketSwitch(): Promise<void> {
    await this.upQueue.loadFromDatabase();
    this.scheduler.resume();
    this.logger.info(
      { Category: 'sp00ky-client::Sp00kySync::completeBucketSwitch' },
      'Sync resumed after bucket switch'
    );
  }

  /**
   * Push the authenticated user's record id from the parent client's
   * auth subscription. Tears down the existing `_00_list_ref` LIVE (if
   * any) and re-registers it under the new user's dedicated table so
   * SurrealDB binds the permission rule under the post-flip auth
   * context. Pass `null` on sign-out.
   *
   * The dedicated `_00_list_ref_user_<id>` table is created lazily by
   * the SSP when the first query registration arrives, which may be
   * concurrent with this call. We retry the LIVE registration with a
   * short backoff so a "table not found" race resolves without
   * surfacing as a permanent auth-loading hang.
   */
  public async setCurrentUserId(userId: string | null): Promise<void> {
    if (this.currentUserId === userId) return;
    this.currentUserId = userId;
    if (!userId) {
      if (this.anonLiveEnabled) {
        // Signed out but anonymous realtime is on: keep the poll running and
        // re-point LIVE from the (now stale) per-user table to the shared
        // `_00_list_ref_anon`. `startListRefPoll` is idempotent; the poll
        // re-resolves `listRefTable()` each tick so it follows automatically.
        this.startListRefPoll();
        await this.restartRefLiveQuery().catch((err) => {
          this.logger.debug(
            { err, Category: 'sp00ky-client::Sp00kySync::setCurrentUserId' },
            'Anonymous ref LIVE restart failed; relying on periodic poll fallback'
          );
        });
        return;
      }
      await this.killRefLiveQuery();
      this.stopListRefPoll();
      return;
    }
    // Start periodic polling FIRST so we have a deterministic fallback
    // even when LIVE registration fails or SurrealDB drops a delivery.
    this.startListRefPoll();
    // Try to start LIVE with backoff for low-latency delivery on the
    // happy path; the poll handles the rest.
    const attemptDelays = [0, 250, 500, 1000, 2000];
    for (let i = 0; i < attemptDelays.length; i++) {
      if (attemptDelays[i] > 0) {
        this._liveRetryCount++;
        await new Promise((r) => setTimeout(r, attemptDelays[i]));
      }
      try {
        await this.restartRefLiveQuery();
        return;
      } catch (err) {
        this.logger.debug(
          { err, attempt: i + 1, Category: 'sp00ky-client::Sp00kySync::setCurrentUserId' },
          'Ref LIVE start failed; relying on periodic poll fallback'
        );
      }
    }
  }

  private startListRefPoll(): void {
    if (this.listRefPollRunning) return;
    this.listRefPollRunning = true;
    this.logger.debug(
      {
        intervalMs: this.refSyncIntervalMs,
        Category: 'sp00ky-client::Sp00kySync::startListRefPoll',
      },
      'list_ref poll loop started'
    );
    const schedule = (delayMs: number) => {
      this.listRefPollTimer = setTimeout(async () => {
        if (!this.listRefPollRunning) return;
        let changed = false;
        const tick = (async () => {
          changed = await this.pollListRefForActiveQueries();
        })();
        this.listRefPollInFlight = tick.catch(() => {});
        try {
          await tick;
        } finally {
          this.listRefPollInFlight = null;
          if (!this.listRefPollRunning) return;
          // Reset the idle streak on any observed change so the poll snaps
          // back to the fast base cadence; otherwise grow it so a quiet page
          // backs off toward the cap. (`handleRemoteListRefChange` also resets
          // it when a LIVE event lands.)
          this.listRefIdleStreak = changed ? 0 : this.listRefIdleStreak + 1;
          const next = listRefPollDelayMs({
            idleStreak: this.listRefIdleStreak,
            baseIntervalMs: this.refSyncIntervalMs,
          });
          schedule(next);
        }
      }, delayMs);
    };
    schedule(this.refSyncIntervalMs);
  }

  private stopListRefPoll(): void {
    this.listRefPollRunning = false;
    if (this.listRefPollTimer !== null) {
      clearTimeout(this.listRefPollTimer);
      this.listRefPollTimer = null;
    }
  }

  /**
   * One poll cycle: refetch `_00_list_ref` for every active query. Returns
   * whether ANY query's remoteArray actually changed — the scheduler uses this
   * to drive the adaptive idle backoff.
   *
   * Also the ONLY health signal that runs while the page is idle. Sync health is
   * otherwise activity-driven (mutations/registrations via the scheduler,
   * reconnect re-registration, self-heal), so on a quiet page a stale `degraded`
   * would linger until the next mutation and a genuine idle drop would be
   * invisible. We fold the cycle's aggregate reachability into `recordSyncOutcome`
   * so idle health self-recovers (and self-degrades) with no user action. A clean
   * cycle is idempotent when already healthy (`recordSyncOutcome` early-returns at
   * `consecutiveSyncFailures === 0`), so a healthy idle page pays nothing.
   */
  private async pollListRefForActiveQueries(): Promise<boolean> {
    const hashes = this.dataModule.getActiveQueryHashes();
    if (hashes.length === 0) {
      // No active queries to piggyback on, but health still needs a heartbeat —
      // probe connectivity directly so an idle page with no live queries doesn't
      // go blind. Cheap, and gated by the same adaptive backoff (≤5s idle cap).
      try {
        await this.remote.query('RETURN true');
        this.recordSyncOutcome(true);
      } catch (err) {
        this.recordSyncOutcome(false, err);
      }
      return false;
    }
    let anyChanged = false;
    // `reached` = the server answered at least once this cycle (a success, or an
    // *application* error, which still proves reachability). `firstNetworkErr`
    // holds the first network-classified failure. A cycle that only produced
    // network errors reports the outcome as a down round; a mixed/app cycle counts
    // as reached; an all-application cycle reports nothing (that's a query-shape
    // fault owned by the registration path, not a reachability signal).
    let reached = false;
    let firstNetworkErr: unknown;
    for (const hash of hashes) {
      try {
        if (await this.refetchListRefForQuery(hash)) anyChanged = true;
        reached = true;
      } catch (err) {
        if (classifySyncError(err) === 'network') {
          if (firstNetworkErr === undefined) firstNetworkErr = err;
        } else {
          reached = true;
        }
        this.logger.debug(
          { err: (err as Error)?.message ?? err, hash, Category: 'sp00ky-client::Sp00kySync::pollListRefForActiveQueries' },
          'Per-query list_ref poll failed'
        );
      }
    }
    // Call the private outcome recorder directly rather than routing through the
    // scheduler — the scheduler only reports on rounds that drained ≥1 queue item
    // (`processedAny`), and this isn't a queue round.
    if (reached) {
      this.recordSyncOutcome(true);
    } else if (firstNetworkErr !== undefined) {
      this.recordSyncOutcome(false, firstNetworkErr);
    }
    return anyChanged;
  }

  /**
   * Pull the upstream list_ref entries for `queryHash`, diff them
   * against the local `remoteArray` cache, sync any added/updated rows
   * through the SyncEngine, then persist the new remoteArray. This is
   * the same shape `createRemoteQuery` does for its initial fetch and
   * what `handleRemoteListRefChange` does per-LIVE-event — we reuse
   * it on a timer as a fallback for missed LIVE notifications.
   */
  private async refetchListRefForQuery(queryHash: string): Promise<boolean> {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return false;
    const listRefTbl = this.listRefTable();
    const [items] = await this.remote.query<[{ out: RecordId<string>; version: number }[]]>(
      buildListRefSelect(listRefTbl),
      { in: queryState.config.id }
    );
    if (!Array.isArray(items)) return false;
    const fresh: RecordVersionArray = items.map((item) => [
      encodeRecordId(item.out),
      item.version,
    ]);
    // Capture which ids LEFT the query's window (present in the cached
    // remoteArray, absent from `fresh`) BEFORE we overwrite remoteArray — these
    // are cross-window deletes (or rows that scrolled out). They drive the
    // forced re-render below.
    const prevRemote = queryState.config.remoteArray ?? [];
    const freshIds = new Set(fresh.map(([id]) => id));
    const removedIds = prevRemote.filter(([id]) => !freshIds.has(id)).map(([id]) => id);
    // Idempotent poll: only persist the remoteArray when it actually changed.
    // The poll runs continuously as a LIVE fallback, so on a quiet page `fresh`
    // equals the cached array every tick — re-writing it (an `UPDATE _00_query`
    // each cycle, per active query) was pure churn and the bulk of the idle
    // traffic. `recordVersionArraysEqual` is order-insensitive because the
    // list_ref SELECT has no `ORDER BY`.
    const changed = !recordVersionArraysEqual(fresh, queryState.config.remoteArray);
    if (changed) {
      // Update the cached remoteArray so the next diff/sync sees the new state.
      // `syncQuery` (below) then writes through `cache.saveBatch`, which UPSERTs
      // the local DB row and ingests it into the in-browser SSP — the SSP's
      // stream updates run `processStreamUpdate`, which re-queries the local DB
      // and notifies subscribers. We skip an explicit `notifyQuerySynced`
      // because that path races the stream-update path (can notify with stale
      // records).
      await this.dataModule.updateQueryRemoteArray(queryHash, fresh);
    }
    // Run `syncQuery` every tick regardless: it's a no-op when localArray has
    // caught up to remoteArray (`if (!diff) return`, issues no query), but it
    // covers the rare case where remoteArray is stable yet localArray is behind
    // (a prior record fetch failed) — so a missed row still gets retried.
    // For REMOVALS it runs the ids through `handleRemovedRecords`, which deletes
    // confirmed-gone records from the local DB.
    try {
      await this.syncQuery(queryHash);
    } catch (err) {
      this.logger.info(
        { err: (err as Error)?.message ?? err, queryHash, Category: 'sp00ky-client::Sp00kySync::refetchListRefForQuery' },
        'syncQuery failed during poll'
      );
    }
    // Cross-session fallback for `.related()` child rows: the LIVE-permission
    // gap can drop child-edge notifications, so converge their bodies on the
    // poll too (idempotent — no-op when nothing changed).
    await this.syncSubqueryChildren(queryHash).catch((err) => {
      this.logger.info(
        { err: (err as Error)?.message ?? err, queryHash, Category: 'sp00ky-client::Sp00kySync::refetchListRefForQuery' },
        'Subquery child sync failed during poll'
      );
    });
    // A REMOVAL needs no record fetch, so unlike the added-row path it doesn't
    // get a re-render from the SSP stream on this code path reliably (and the
    // non-windowed window-0 query re-queries the local DB rather than the id-set).
    // Force a re-materialize + notify so the deleted row drops from the list in
    // this (second) window — the reliable, LIVE-independent cross-window path.
    if (removedIds.length > 0) {
      try {
        await this.dataModule.notifyQuerySynced(queryHash);
      } catch (err) {
        this.logger.info(
          { err: (err as Error)?.message ?? err, queryHash, Category: 'sp00ky-client::Sp00kySync::refetchListRefForQuery' },
          'notifyQuerySynced failed during poll-removal re-render'
        );
      }
    }
    return changed;
  }

  /**
   * Resolve the current `_00_list_ref` table name for the active auth
   * context. Public so the `createRemoteQuery` initial-fetch path can
   * read from the right per-user table.
   *
   * Reads the user id from `DataModule` rather than the local mirror,
   * because `DataModule.setCurrentUserId` runs synchronously from the
   * auth callback (before any `await`), whereas `sync.setCurrentUserId`
   * is async — the userQuery's initial fetch can fire between those
   * two points and we need the correct table name immediately.
   */
  public listRefTable(): string {
    const userId = this.dataModule.getCurrentUserId();
    // Unauthenticated with the flag on → the shared `_00_list_ref_anon` table.
    if (userId == null && this.anonLiveEnabled) {
      return listRefTableFor(this.refMode, ANON_USER_ID);
    }
    return listRefTableFor(this.refMode, userId);
  }

  private async killRefLiveQuery(): Promise<void> {
    if (this.liveQueryUnsubscribe) {
      try { this.liveQueryUnsubscribe(); } catch { /* ignore */ }
      this.liveQueryUnsubscribe = null;
    }
    if (this.currentLiveQueryUuid !== null) {
      try {
        await this.remote.query('KILL $u', { u: this.currentLiveQueryUuid });
      } catch (err) {
        this.logger.debug(
          { err, Category: 'sp00ky-client::Sp00kySync::killRefLiveQuery' },
          'Prior LIVE KILL failed; continuing'
        );
      }
      this.currentLiveQueryUuid = null;
    }
  }

  private async restartRefLiveQuery(): Promise<void> {
    await this.killRefLiveQuery();
    await this.startRefLiveQueries();
  }

  // Only the connect that follows a prior disconnect counts as a
  // reconnect; the initial connect after init() must not trigger a
  // refetch storm.
  private subscribeToReconnect() {
    const client = this.remote.getClient();
    client.subscribe('disconnected', () => {
      this.wasDisconnected = true;
      this.logger.info(
        { Category: 'sp00ky-client::Sp00kySync::onDisconnect' },
        'Remote disconnected'
      );
    });
    client.subscribe('connected', () => {
      if (!this.wasDisconnected) return;
      this.wasDisconnected = false;
      this.logger.info(
        { Category: 'sp00ky-client::Sp00kySync::onReconnect' },
        'Remote reconnected, refetching active queries'
      );
      for (const hash of this.dataModule.getActiveQueryHashes()) {
        this.scheduler.enqueueDownEvent({ type: 'register', payload: { hash } });
      }
      // The WS reconnect leaves the server-side LIVE subscription dead — the
      // re-enqueued `register` events only re-fetch initial state, they don't
      // re-subscribe. Without this, LIVE never recovers after a reconnect and
      // the poll silently becomes the sole sync path (and never backs off).
      // Authenticated → per-user table; signed-out with anon live enabled →
      // the shared `_00_list_ref_anon`. Otherwise there's no table to re-bind.
      if (this.currentUserId || this.anonLiveEnabled) {
        this.restartRefLiveQuery().catch((err) => {
          this.logger.debug(
            { err, Category: 'sp00ky-client::Sp00kySync::onReconnect' },
            'LIVE restart after reconnect failed; relying on poll fallback'
          );
        });
      }
    });
  }

  private async startRefLiveQueries() {
    const tableName = this.listRefTable();
    this.logger.debug(
      { tableName, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
      'Starting ref live queries'
    );

    const [queryUuid] = await this.remote.query<[Uuid]>(
      `LIVE SELECT * FROM ${tableName}`
    );
    this.currentLiveQueryUuid = queryUuid;

    const live = await this.remote.getClient().liveOf(queryUuid);
    this.liveQueryUnsubscribe = live.subscribe((message) => {
      this.logger.debug(
        { message, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
        'Live update received'
      );
      if (message.action === 'KILLED') return;
      // Subquery child edges (rows with `parent` set) are NOT primary window
      // rows — the client's `RecordVersionArray` only tracks primary rows, so
      // routing them through `handleRemoteListRefChange` would surface them as
      // spurious "added" diffs and pollute the window. Instead route them to
      // the dedicated child-body sync so `.related()` data stays realtime
      // cross-session (the LIVE-permission gap otherwise leaves it to the poll).
      if ((message.value as { parent?: unknown }).parent != null) {
        this.handleRemoteSubqueryChange(
          message.action,
          message.value.in as RecordId<string>,
          message.value.out as RecordId<string>,
          message.value.version as number
        ).catch((err) => {
          this.logger.error(
            { err, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
            'Error handling remote subquery change'
          );
        });
        return;
      }
      this.handleRemoteListRefChange(
        message.action,
        message.value.in as RecordId<string>,
        message.value.out as RecordId<string>,
        message.value.version as number
      ).catch((err) => {
        this.logger.error(
          { err, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
          'Error handling remote list ref change'
        );
      });
    });
  }

  private async handleRemoteListRefChange(
    action: 'CREATE' | 'UPDATE' | 'DELETE',
    queryId: RecordId,
    recordId: RecordId,
    version: number
  ) {
    // Any LIVE delivery is evidence of activity — a CREATE/UPDATE/DELETE on a
    // query's window, or a notification for an unknown local query. Reset the
    // poll's idle streak so it snaps back to the fast base cadence (the page
    // is clearly not idle), and record the timestamp as a liveness diagnostic.
    this.lastLiveEventAt = Date.now();
    this.listRefIdleStreak = 0;

    // NOTE: DELETE is handled like CREATE/UPDATE below. When another window (or
    // this one) deletes a record, the server's SSP removes it from `_00_list_ref`
    // and the LIVE subscription delivers a DELETE here — `createDiffFromDbOp`
    // turns it into a `removed: [recordId]` diff so the row drops from the window
    // in realtime. (It was previously ignored, so other windows only caught up on
    // reload / the slow poll.)
    const existing = this.dataModule.getQueryById(queryId);

    if (!existing) {
      this.logger.warn(
        {
          queryId: queryId.toString(),
          Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
        },
        'Received remote update for unknown local query'
      );
      return;
    }

    const { localArray } = existing.config;

    this.logger.debug(
      {
        action,
        queryId,
        recordId,
        version,
        localArray,
        Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
      },
      'Live update is being processed'
    );
    const diff = createDiffFromDbOp(action, recordId, version, localArray);
    // `config.id` is `_00_query:<hash>`, so its id-part IS the query hash
    // (a SHA-256 over query content + sessionId) — the key DataModule uses.
    const hash = extractIdPart(existing.config.id);
    await this.runSyncForQuery(hash, diff);
  }

  /**
   * Handle a LIVE change to a SUBQUERY child edge (a `_00_list_ref` row with
   * `parent` set) for a `.related()` query. Unlike primary rows, child rows
   * must NOT touch the query's `localArray`/`remoteArray`/`rowCount`; we only
   * keep the child BODY fresh in the local cache so the in-browser SSP's
   * subquery-table dependency re-materializes the parent view.
   *
   * CREATE/UPDATE fetch+upsert the child body. DELETE is intentionally a
   * no-op: a child leaving this query's set must not delete a body another
   * query may still show (see `syncSubqueryChildren` deletion-safety note);
   * a genuine record delete propagates via the normal delete path.
   */
  private async handleRemoteSubqueryChange(
    action: 'CREATE' | 'UPDATE' | 'DELETE',
    queryId: RecordId,
    childId: RecordId,
    version: number
  ) {
    this.lastLiveEventAt = Date.now();
    this.listRefIdleStreak = 0;

    if (action === 'DELETE') return;

    const existing = this.dataModule.getQueryById(queryId);
    if (!existing) return;

    const item = { id: childId, version };
    await this.syncEngine.syncRecords(
      action === 'CREATE'
        ? { added: [item], updated: [], removed: [] }
        : { added: [], updated: [item], removed: [] }
    );

    // Keep the in-memory child array in step so the poll's idempotent diff
    // doesn't re-fetch this body on the next tick.
    const key = encodeRecordId(childId);
    const prev = existing.config.subqueryRemoteArray ?? [];
    existing.config.subqueryRemoteArray = [...prev.filter(([id]) => id !== key), [key, version]];
  }

  /**
   * Enqueues a 'down' event (from remote to local) for processing.
   * @param event The DownEvent to enqueue.
   */
  public enqueueDownEvent(event: DownEvent) {
    this.scheduler.enqueueDownEvent(event);
  }

  private async processUpEvent(event: UpEvent) {
    this.logger.debug(
      { event, Category: 'sp00ky-client::Sp00kySync::processUpEvent' },
      'Processing up event'
    );
    switch (event.type) {
      case 'create': {
        const dataKeys = Object.keys(event.data).map((key) => ({ key, variable: `data_${key}` }));
        const prefixedParams = Object.fromEntries(
          dataKeys.map(({ key, variable }) => [variable, event.data[key]])
        );
        const query = surql.seal(surql.createSet('id', dataKeys));
        await this.remote.query(query, {
          id: event.record_id,
          ...prefixedParams,
        });
        break;
      }
      case 'update':
        await this.remote.query(`UPDATE $id MERGE $data`, {
          id: event.record_id,
          data: event.data,
        });
        break;
      case 'delete':
        await this.remote.query(`DELETE $id`, {
          id: event.record_id,
        });
        break;
      default:
        this.logger.error(
          { event, Category: 'sp00ky-client::Sp00kySync::processUpEvent' },
          'processUpEvent unknown event type'
        );
        return;
    }
  }

  private async handleRollback(event: UpEvent, error: Error): Promise<void> {
    const recordId = encodeRecordId(event.record_id);
    const tableName =
      event.type === 'create' && event.tableName
        ? event.tableName
        : extractTablePart(recordId);

    this.logger.warn(
      {
        type: event.type,
        recordId,
        tableName,
        error: error.message,
        Category: 'sp00ky-client::Sp00kySync::handleRollback',
      },
      'Rolling back failed mutation'
    );

    switch (event.type) {
      case 'create':
        await this.dataModule.rollbackCreate(event.record_id, tableName);
        break;
      case 'update':
        if (event.beforeRecord) {
          await this.dataModule.rollbackUpdate(event.record_id, tableName, event.beforeRecord);
        } else {
          this.logger.warn(
            {
              recordId,
              Category: 'sp00ky-client::Sp00kySync::handleRollback',
            },
            'Cannot rollback update: no beforeRecord available. Down-sync will reconcile.'
          );
        }
        break;
      case 'delete':
        this.logger.warn(
          {
            recordId,
            Category: 'sp00ky-client::Sp00kySync::handleRollback',
          },
          'Delete rollback not implemented. Down-sync will reconcile.'
        );
        break;
    }

    this.events.emit(SyncEventTypes.MutationRolledBack, {
      eventType: event.type,
      recordId,
      error: error.message,
    });
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug(
      { event, Category: 'sp00ky-client::Sp00kySync::processDownEvent' },
      'Processing down event'
    );
    switch (event.type) {
      case 'register':
        return this.registerQuery(event.payload.hash);
      case 'sync':
        return this.syncQuery(event.payload.hash);
      case 'heartbeat':
        return this.heartbeatQuery(event.payload.hash);
      case 'cleanup':
        return this.cleanupQuery(event.payload.hash);
    }
  }

  /**
   * Synchronizes a specific query by hash.
   * Compares local and remote version arrays and fetches differences.
   * @param hash The hash of the query to sync.
   */
  public async syncQuery(hash: string) {
    const queryState = this.dataModule.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn(
        { hash, Category: 'sp00ky-client::Sp00kySync::syncQuery' },
        'Query not found'
      );
      return;
    }

    const diff = new ArraySyncer(
      queryState.config.localArray,
      queryState.config.remoteArray
    ).nextSet();

    if (!diff) {
      return;
    }
    return this.runSyncForQuery(hash, diff);
  }

  /**
   * Run a sync for a single query while reflecting its fetch status. Marks the
   * query `fetching` for the duration when the diff actually pulls records
   * (added/updated), then resets to `idle` in a `finally` so a failed sync
   * never leaves a query stuck `fetching`. Part A's notification coalescing
   * means the single resulting UI update lands after this completes.
   */
  private async runSyncForQuery(hash: string, diff: RecordVersionDiff): Promise<void> {
    // Don't let sync re-add a record the user just deleted locally. The remote
    // delete is queued in the outbox, so until it's processed the server's
    // `_00_list_ref` still lists the record — the diff then classifies it as
    // `added` (present remotely, absent locally) and `syncRecords` re-fetches +
    // re-inserts it, so a deleted database reappears a few seconds later. Drop
    // any id with a pending local DELETE from the re-add paths. Once the remote
    // delete lands, the pending row clears and the server drops it from
    // `_00_list_ref`, so this guard naturally stops applying.
    if (diff.added.length > 0 || diff.updated.length > 0) {
      const pendingDeletes = await this.getPendingDeleteIds();
      if (pendingDeletes.size > 0) {
        diff = {
          added: diff.added.filter((r) => !pendingDeletes.has(encodeRecordId(r.id))),
          updated: diff.updated.filter((r) => !pendingDeletes.has(encodeRecordId(r.id))),
          removed: diff.removed,
        };
      }
    }

    const fetching = diff.added.length + diff.updated.length > 0;
    if (fetching) {
      this.dataModule.beginFetching(hash);
    }
    try {
      const { remoteFetchMs, stillRemoteIds } = await this.syncEngine.syncRecords(diff);
      if (fetching) {
        this.dataModule.recordRemoteFetch(hash, remoteFetchMs);
      }
      // Converge localArray to the authoritative remoteArray for ids that left
      // the server's list_ref but still exist — a view-membership change, not a
      // delete — so the poll's diff stops re-flagging them every tick (the `job:`
      // churn). CRUCIAL: only converge after the id has been still-remote for
      // several CONSECUTIVE rounds. A record that's merely mid-deletion is
      // still-remote for ~one round (its delete hasn't committed when our
      // existence check races it) and is gone the next round → it never reaches
      // the threshold, so it's deleted normally instead of being stranded here.
      if (stillRemoteIds.length > 0) {
        const CONVERGE_AFTER = 3;
        const toConverge: string[] = [];
        for (const id of stillRemoteIds) {
          const key = `${hash}:${id}`;
          const n = (this.stillRemoteStreaks.get(key) ?? 0) + 1;
          if (n >= CONVERGE_AFTER) {
            this.stillRemoteStreaks.delete(key);
            toConverge.push(id);
          } else {
            this.stillRemoteStreaks.set(key, n);
          }
        }
        if (toConverge.length > 0) {
          const qs = this.dataModule.getQueryByHash(hash);
          const local = qs?.config.localArray;
          if (local && local.length > 0) {
            const drop = new Set(toConverge);
            const next = local.filter(([id]) => !drop.has(id));
            if (next.length !== local.length) {
              await this.dataModule.updateQueryLocalArray(hash, next);
            }
          }
        }
      }
    } finally {
      if (fetching) {
        // Land the coalesced result BEFORE flipping to idle: the final stream
        // update sits on a debounce timer, and an `idle` that races ahead of it
        // would let consumers treat a partially-filled window as authoritative.
        try {
          await this.dataModule.flushPendingStreamUpdate(hash);
        } catch (err) {
          this.logger.warn(
            { err, hash, Category: 'sp00ky-client::Sp00kySync::runSyncForQuery' },
            'Failed to flush pending stream update before idle'
          );
        }
        this.dataModule.endFetching(hash);
      }
    }
  }

  /**
   * Record ids with a pending local DELETE in the outbox (`_00_pending_mutations`).
   * Sync must not re-fetch/re-insert these — the remote delete is async, so the
   * server's `_00_list_ref` still lists them until it's processed, and the diff
   * would otherwise resurrect a just-deleted record.
   */
  private async getPendingDeleteIds(): Promise<Set<string>> {
    try {
      const [rows] = await this.local.query<[{ recordId: RecordId<string> }[]]>(
        "SELECT recordId FROM _00_pending_mutations WHERE mutationType = 'delete'"
      );
      return new Set((rows ?? []).map((r) => encodeRecordId(r.recordId)));
    } catch (err) {
      this.logger.warn(
        { err, Category: 'sp00ky-client::Sp00kySync::getPendingDeleteIds' },
        'Failed to read pending deletes; sync may briefly resurrect a just-deleted record'
      );
      return new Set();
    }
  }

  /**
   * Enqueues a list of mutations (up events) to be sent to the remote.
   * @param mutations Array of UpEvents (create/update/delete) to enqueue.
   */
  public async enqueueMutation(mutations: UpEvent[]) {
    this.scheduler.enqueueMutation(mutations);
  }

  private async registerQuery(queryHash: string) {
    // Hold `fetching` across the WHOLE registration (remote view creation +
    // initial sync + post-sync notify). A query is born `fetching` in
    // createNewQuery; this refcounted cycle is what resolves it to `idle` — so
    // consumers (e.g. useQuery's `isSettled`) never see an idle query whose
    // window is still empty/partially materialized.
    this.dataModule.beginFetching(queryHash);
    try {
      this.logger.debug(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'Register Query state'
      );
      await this.createRemoteQuery(queryHash);
      await this.syncQuery(queryHash);
      // Land any still-debounced stream result, then always notify — handles
      // empty result sets where no stream updates fire but the UI needs to
      // stop loading.
      await this.dataModule.flushPendingStreamUpdate(queryHash);
      await this.dataModule.notifyQuerySynced(queryHash);
    } catch (e) {
      this.logger.error(
        { err: e, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'registerQuery error'
      );
      throw e;
    } finally {
      this.dataModule.endFetching(queryHash);
    }
  }

  private async createRemoteQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);

    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::createRemoteQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    // Delegate to remote function which handles DBSP registration & persistence.
    // clientId is set server-side from session::id() — see fn::query::register.
    await this.remote.query('fn::query::register($config)', {
      config: {
        id: queryState.config.id,
        surql: queryState.config.surql,
        params: queryState.config.params,
        ttl: queryState.config.ttl,
      },
    });

    // Initial materialized-view fetch — pull from the same per-user
    // `_00_list_ref_user_<id>` (or global `_00_list_ref` in single
    // mode) that the LIVE subscription listens on, so the two stay in
    // sync. `parent IS NONE` excludes subquery entries; the
    // `localArray` cache only tracks primary records.
    const listRefTbl = this.listRefTable();
    const [items] = await this.remote.query<[{ out: RecordId<string>; version: number }[]]>(
      buildListRefSelect(listRefTbl),
      {
        in: queryState.config.id,
      }
    );

    this.logger.trace(
      {
        queryId: encodeRecordId(queryState.config.id),
        items,
        Category: 'sp00ky-client::Sp00kySync::createRemoteQuery',
      },
      'Got query record version array from remote'
    );

    const array: RecordVersionArray = items.map((item) => [encodeRecordId(item.out), item.version]);

    this.logger.debug(
      {
        queryId: encodeRecordId(queryState.config.id),
        array,
        Category: 'sp00ky-client::Sp00kySync::createRemoteQuery',
      },
      'createdRemoteQuery'
    );

    if (array) {
      /// Incantation existed already
      await this.dataModule.updateQueryRemoteArray(queryHash, array);
    }

    // Pull the bodies of any `.related()` subquery children into the local
    // cache. The primary fetch above (`parent IS NONE`) tracks only window
    // rows, so without this a cold-reload re-materialization of the
    // correlated surql finds no child rows and related fields come back
    // empty. Best-effort: never fail registration over it.
    await this.syncSubqueryChildren(queryHash).catch((err) => {
      this.logger.info(
        { err: (err as Error)?.message ?? err, queryHash, Category: 'sp00ky-client::Sp00kySync::createRemoteQuery' },
        'Subquery child sync failed during registration; poll will retry'
      );
    });
  }

  /**
   * Sync the BODIES of a `.related()` query's subquery child rows into the
   * local cache, separately from the primary window array. The SSP writes
   * each matched child as a `_00_list_ref` edge tagged `parent`/`parent_rel`;
   * `buildSubqueryListRefSelect` pulls those `out`+`version` pairs (any
   * nesting depth). We diff against the in-memory `subqueryRemoteArray` and
   * fetch added/updated bodies through the SyncEngine — which `saveBatch`s
   * them into the local DB AND the in-browser SSP, whose subquery-table
   * dependency then re-materializes the parent view (no explicit notify).
   *
   * Deletion safety: we pass `removed: []` deliberately. A child body can be
   * shared by other queries; letting `handleRemovedRecords` delete one that
   * merely left THIS query's child set would clobber data another query still
   * shows. Genuine record deletes flow through the normal delete path; a
   * lingering orphan body is invisible (the correlated WHERE stops matching).
   *
   * Kept off `runSyncForQuery` on purpose so child fetches never flip the
   * query to `fetching` or skew its DevTools timings.
   */
  private async syncSubqueryChildren(queryHash: string): Promise<void> {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return;

    const listRefTbl = this.listRefTable();
    const [items] = await this.remote.query<[{ out: RecordId<string>; version: number }[]]>(
      buildSubqueryListRefSelect(listRefTbl),
      { in: queryState.config.id }
    );
    if (!Array.isArray(items)) return;

    const fresh: RecordVersionArray = items.map((item) => [encodeRecordId(item.out), item.version]);
    const prev = queryState.config.subqueryRemoteArray ?? [];
    if (recordVersionArraysEqual(fresh, prev)) return; // idempotent: nothing new

    const diff = diffRecordVersionArray(prev, fresh);
    if (diff.added.length > 0 || diff.updated.length > 0) {
      await this.syncEngine.syncRecords({
        added: diff.added,
        updated: diff.updated,
        removed: [], // never delete child bodies here — see method doc
      });
    }
    // In-memory only — child rows must never enter the persisted primary array.
    queryState.config.subqueryRemoteArray = fresh;
  }

  public async heartbeatQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::heartbeatQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    await this.remote.query('fn::query::heartbeat($id)', {
      id: queryState.config.id,
    });
  }

  // Eager teardown of a deregistered query's remote `_00_query` view (opt-in,
  // e.g. a viewport-windowed list cancelling an off-screen window). Query ids
  // are a deterministic hash of (surql+params), so a DELETE racing a scroll-back
  // re-register (same id) could nuke a freshly-recreated view — hence two
  // guards: abort if a subscriber reappeared BEFORE the delete; re-register if
  // one reappears DURING the delete's network await. Tolerant of a
  // missing/already-gone query (no throw).
  private async cleanupQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return; // already torn down / never registered

    // Re-subscribed before the queued cleanup ran → keep everything as-is.
    if (this.dataModule.hasSubscribers(queryHash)) return;

    await this.remote.query(`DELETE $id`, { id: queryState.config.id });

    // Re-subscribed while we awaited the DELETE → the remote view is now gone
    // but a subscriber needs it; recreate it instead of leaving a zombie.
    if (this.dataModule.hasSubscribers(queryHash)) {
      this.enqueueDownEvent({ type: 'register', payload: { hash: queryHash } });
      return;
    }

    // No subscribers throughout → safe to free the local view + state.
    this.dataModule.finalizeDeregister(queryHash);
  }
}
