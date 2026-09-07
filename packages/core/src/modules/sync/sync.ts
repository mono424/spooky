import type {
  ConnectionSupervisor,
  LocalStore,
  RemoteDatabaseService,
} from '../../services/database/index';
import type {
  ConnectionState,
  RecordVersionArray,
  RecordVersionDiff,
  SyncHealth,
  SyncHealthStatus,
} from '../../types';
import { createSyncEventSystem, SyncEventTypes, SyncQueueEventTypes } from './events/index';
import type { Logger } from '../../services/logger/index';
import type { DownEvent, UpEvent } from './queue/index';
import { DownQueue, UpQueue } from './queue/index';
import type { RecordId, Uuid } from 'surrealdb';
import {
  applyRecordVersionDiff,
  ArraySyncer,
  buildListRefBatchSelect,
  buildListRefSelect,
  buildQueryRowCountBatchSelect,
  buildQueryRowCountSelect,
  buildSubqueryListRefSelect,
  createDiffFromDbOp,
  diffRecordVersionArray,
  listRefPollDelayMs,
  planListRefPollChunks,
  recordVersionArraysEqual,
  resolveListRefPollInterval,
} from './utils';
import { SyncEngine } from './engine';

/** One query's server-side `_00_list_ref` state as read by the poll. */
interface ListRefSnapshot {
  primary: RecordVersionArray;
  subquery: RecordVersionArray;
  /** `_00_query.rowCount`; `null` when the row was not readable. */
  rowCount: number | null;
}

interface ListRefEdgeRow {
  in: RecordId<string>;
  out: RecordId<string>;
  version: number;
  parent?: unknown;
}
import { SyncScheduler } from './scheduler';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { CacheModule } from '../cache/index';
import type { DataModule } from '../data/index';
import {
  classifySyncError,
  encodeRecordId,
  extractIdPart,
  extractTablePart,
  surql,
  withTimeout,
} from '../../utils/index';
import { ANON_USER_ID, DEFAULT_REF_MODE, listRefTableFor, RefMode } from '../ref-tables';
import { mutationOwnerTabId } from '../data/mutation-id';
import type { LeaderSyncHub, SyncForwarder } from '../../services/tabs/coordinator';
import type { IngestTuple } from '../../services/tabs/protocol';
import { parseRecordIdString } from '../../utils/index';

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
  /**
   * Max time a single mutation push may take before it is treated as a network
   * failure and retried. Guards against an RPC that never settles wedging the
   * up-queue for the session. Defaults to 30000; `0` disables the timeout.
   */
  pushTimeoutMs?: number;
  /**
   * Max time a single down event (`register`/`sync`/`cleanup`) may take before
   * it is treated as a network failure and retried. The mirror of
   * {@link pushTimeoutMs} for the read side, which had no such guard: a
   * `fn::query::register` that never settled held its slot in the down drain,
   * and every later registration behind it, for the rest of the session.
   * Defaults to 30000; `0` disables the timeout.
   */
  downTimeoutMs?: number;
  /**
   * Transport supervisor. Sync reads its state to report `connection` in
   * {@link SyncHealth} so a UI can show "reconnecting…" the instant the socket
   * drops, without waiting for the degrade threshold. Optional: omitted in
   * tests, where `connection` then reports `connected`.
   */
  connectionSupervisor?: ConnectionSupervisor;
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
  /**
   * Set by any event that means the socket we registered on is gone, so the
   * next `connected` knows it must re-subscribe rather than treat itself as the
   * initial connect. See {@link subscribeToReconnect}.
   */
  private needsResubscribe: boolean = false;
  /** When the last reconnect-driven full refetch ran, for burst coalescing. */
  private lastReconnectRefetchAt = 0;
  /**
   * Minimum gap between reconnect-driven full refetches. Long enough to absorb
   * a flapping socket (the SDK reconnect ladder starts at 1s), short enough
   * that a genuine drop minutes later still refetches.
   */
  private static readonly RECONNECT_REFETCH_COOLDOWN_MS = 10_000;
  /** Poll interval while waiting for a reconnected session to re-authenticate. */
  private static readonly AUTH_READY_RETRY_MS = 500;
  /** Attempts before giving up on re-auth and skipping the refetch entirely. */
  private static readonly AUTH_READY_MAX_ATTEMPTS = 10;
  public events = createSyncEventSystem();

  // Auth identity that drives per-user `_00_list_ref_user_<id>` routing
  // in `RefMode.Dedicated`. Updated by `setCurrentUserId` from the auth
  // subscription in `Sp00kyClient`; null when unauthenticated.
  private currentUserId: string | null = null;

  // ---- shared-tabs role state ----
  // Followers keep their own remote WS (registration, per-query sync, poll)
  // but must never drain the shared outbox or hold a second list_ref LIVE.
  // The leader relays its LIVE events and routes rollbacks by mutation owner.
  private tabRole: 'solo' | 'leader' | 'follower' = 'solo';
  private tabId: string | null = null;
  private hub: LeaderSyncHub | null = null;
  private forwarder: SyncForwarder | null = null;

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

  // `queryHash` -> wall-clock ms of its last poll refresh. Drives the per-cycle
  // plan (`planListRefPollChunks`): oldest first, large views less often.
  private listRefPolledAt: Map<string, number> = new Map();

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
    const id1 = this.upQueue.events.subscribe(SyncQueueEventTypes.MutationEnqueued, (event) =>
      cb(event.payload.queueSize)
    );
    const id2 = this.upQueue.events.subscribe(SyncQueueEventTypes.MutationDequeued, (event) =>
      cb(event.payload.queueSize)
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
  /** Per-push RPC deadline; see {@link withPushTimeout}. */
  private readonly pushTimeoutMs: number;
  private readonly downTimeoutMs: number;
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

  /**
   * Transport supervisor, when one was supplied. Sync only reads state from it;
   * it never drives reconnects itself.
   */
  private readonly connectionSupervisor?: ConnectionSupervisor;
  /**
   * Mirror of the supervisor's state. Defaults to `connected` so a client
   * constructed without a supervisor (tests, embedders) reports the same health
   * shape it always has rather than a permanent false "disconnected".
   */
  private connectionState: ConnectionState = 'connected';

  /** Current sync-health snapshot. */
  get syncHealth(): SyncHealth {
    return {
      status: this.syncHealthStatus,
      consecutiveFailures: this.consecutiveSyncFailures,
      kind: this.syncHealthStatus === 'degraded' ? this.lastSyncErrorKind : undefined,
      error: this.syncHealthStatus === 'degraded' ? this.lastSyncErrorMessage : undefined,
      everConnected: this.hasSyncedOnce,
      connection: this.connectionState,
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
   * Mirror the supervisor's transport state into {@link SyncHealth} and emit on
   * every change, so a UI can react to a dropped socket immediately instead of
   * waiting for `degradeAfterFailures` failed rounds. `status` is untouched:
   * a brief reconnect is not a degradation.
   *
   * No explicit unsubscribe: the supervisor is owned by the same client and
   * drops all subscribers in its own `dispose()`, which `Sp00kyClient.close()`
   * calls first.
   */
  private subscribeToConnectionState(): void {
    if (!this.connectionSupervisor) return;
    this.connectionSupervisor.subscribe((state) => {
      if (this.connectionState === state) return;
      this.connectionState = state;
      this.emitSyncHealth();
    });
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
        {
          attempt: this.selfHealAttempts,
          delayMs: delay,
          Category: 'sp00ky-client::Sp00kySync::selfHeal',
        },
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
          // Same identity gate as the reconnect path: self-heal fires while
          // sync is degraded, which is exactly when a session is most likely
          // to have lost its `$auth`, and a register issued there stamps the
          // view with an empty identity permanently. See
          // `remoteAuthEstablished`.
          const hashes = this.dataModule.getActiveQueryHashes();
          const canRegister = hashes.length > 0 && (await this.remoteAuthEstablished());
          if (canRegister) {
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

  /**
   * Release a deregistered query's remote view immediately instead of leaving
   * it to the TTL sweep. Off by default; see the reasoning in
   * {@link cleanupQuery}. Kept as a field rather than deleted so the eager path
   * can be re-enabled in a test once the subquery-body repair path exists.
   */
  private readonly releaseQueriesEagerly = false;

  constructor(
    private local: LocalStore,
    private remote: RemoteDatabaseService,
    private cache: CacheModule,
    private dataModule: DataModule<S>,
    private schema: S,
    logger: Logger,
    options?: Sp00kySyncOptions
  ) {
    this.logger = logger.child({ service: 'Sp00kySync' });
    this.upQueue = new UpQueue(this.local, this.logger, (dropped) =>
      this.onMutationDropped(dropped)
    );
    this.downQueue = new DownQueue(this.local, this.logger);
    this.syncEngine = new SyncEngine(this.remote, this.cache, this.schema, this.logger);
    this.scheduler = new SyncScheduler(
      this.upQueue,
      this.downQueue,
      this.processUpEvent.bind(this),
      this.processDownEvent.bind(this),
      this.logger,
      this.handleRollback.bind(this),
      this.recordSyncOutcome.bind(this),
      this.handleMutationSettled.bind(this)
    );
    this.refSyncIntervalMs = resolveListRefPollInterval(options?.refSyncIntervalMs);
    this.anonLiveEnabled = options?.anonymousLiveQueries ?? false;
    this.degradeAfterFailures = Math.max(0, options?.degradeAfterConsecutiveFailures ?? 3);
    this.pushTimeoutMs = Math.max(0, options?.pushTimeoutMs ?? 30_000);
    this.downTimeoutMs = Math.max(0, options?.downTimeoutMs ?? 30_000);
    this.connectionSupervisor = options?.connectionSupervisor;
  }

  /**
   * Initializes the synchronization system.
   * Starts the scheduler and initiates the initial sync cycles.
   * @throws Error if already initialized.
   */
  public async init() {
    if (this.isInit) throw new Error('Sp00kySync is already initialized');
    this.isInit = true;
    await this.scheduler.init({ loadOutbox: this.tabRole !== 'follower' });
    // Boot is local-first now, so init() routinely runs BEFORE the socket is
    // up. Treat that as "the socket we registered on is gone": otherwise the
    // first `connected` takes the initial-connect branch, returns early, and
    // never re-enqueues `register` for queries that registered while offline.
    // They would still heal via the down-queue backoff, but slowly and only
    // because every query happens to enqueue its own register.
    if (this.remote.getStatus() !== 'connected') this.needsResubscribe = true;
    this.subscribeToReconnect();
    this.subscribeToConnectionState();
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

  // ---- shared-tabs roles ------------------------------------------------------

  /** Set BEFORE init(): shapes what init boots (a follower loads no outbox and
   *  never starts LIVE; its own registration/poll paths stay untouched). */
  public setTabContext(role: 'solo' | 'leader' | 'follower', tabId: string | null): void {
    this.tabRole = role;
    this.tabId = tabId;
  }

  /** In-flight {@link resumeLeaderDuties}, so a second call joins the first
   *  instead of double-draining the outbox. */
  private leaderDutiesInFlight: Promise<void> | null = null;

  /** Resolves once the in-browser circuit has been primed from the local
   *  store. Every sync diff waits on it: diffing against an empty circuit
   *  classifies the whole working set as missing and re-downloads it. */
  private primeGate: () => Promise<void> = () => Promise.resolve();
  /** The prime we last waited on. `whenPrimed` hands out one promise per
   *  prime, so a new identity means a new prime (boot, bucket switch) ran. */
  private settledPrime: Promise<void> | null = null;

  setPrimeGate(gate: () => Promise<void>): void {
    this.primeGate = gate;
    this.settledPrime = null;
  }

  /**
   * Leader WIRING only, and deliberately synchronous.
   *
   * The coordinator publishes the leader role and tells the broker
   * `leader-ready` the moment the store is adopted, and the broker can mint a
   * follower's ports on the very next tick. So the follower-message handler
   * has to be live before this returns, or a mutation forwarded in that window
   * is dropped. Everything that can block (outbox reload, LIVE restart) moved
   * to {@link resumeLeaderDuties}: a promotion that waits on the network holds
   * `leader-ready` back, and a broker whose leader never reports ready serves
   * no follower ports and re-elects no one, which wedges the whole namespace.
   */
  public promoteToLeader(hub: LeaderSyncHub): void {
    this.tabRole = 'leader';
    this.hub = hub;
    this.forwarder = null;
    this.leaderDutiesInFlight = null;
    hub.onFollowerMessage = (tabId, msg) => {
      switch (msg.type) {
        case 'sync-hello':
          break;
        case 'mutation-enqueued':
          // A write is activity: snap the poll back to its base cadence so the
          // membership for it lands fast even if LIVE drops the event.
          this.listRefIdleStreak = 0;
          void this.enqueueForwardedMutation(msg.mutationId);
          break;
        case 'ingest':
          // A follower's optimistic write. The row is already in the shared
          // store; feed this tab's circuit and fan it out to every OTHER
          // follower, so the write shows up everywhere in one hop instead of
          // after the server round-trip (which also depends on LIVE delivery).
          this.applyRelayedIngest(msg.tuples);
          hub.relayIngest(msg.tuples, tabId);
          this.listRefIdleStreak = 0;
          break;
        case 'request-poll':
          this.listRefIdleStreak = 0;
          break;
      }
    };
  }

  /** Leader duties: drain the shared outbox, own the single list_ref LIVE,
   *  relay LIVE events and rollbacks to followers. Idempotent for a boot-time
   *  leader; a runtime promotion (failover) reloads the outbox, which now
   *  holds EVERY tab's rows, and restarts LIVE under this session. Runs in the
   *  background off the promotion path, so however long it takes (or if it
   *  never finishes) the tab is already a working leader. */
  public resumeLeaderDuties(): Promise<void> {
    if (this.leaderDutiesInFlight) return this.leaderDutiesInFlight;
    const run = (async () => {
      if (!this.isInit || this.tabRole !== 'leader') return;
      await this.upQueue.loadFromDatabase();
      void this.scheduler.syncUp();
      if (this.currentUserId || this.anonLiveEnabled) {
        this.startListRefPoll();
        await this.restartRefLiveQuery().catch((err) => {
          this.logger.warn(
            { err, Category: 'sp00ky-client::Sp00kySync::resumeLeaderDuties' },
            'LIVE restart failed on promotion; poll fallback covers it'
          );
        });
      }
    })();
    this.leaderDutiesInFlight = run;
    return run.finally(() => {
      if (this.leaderDutiesInFlight === run) this.leaderDutiesInFlight = null;
    });
  }

  /** Follower duties: no outbox drain, no LIVE. Mutations forward to the
   *  leader; everything else (registration, per-query sync, poll) runs
   *  against this tab's own remote session as usual. */
  public demoteToFollower(forwarder: SyncForwarder): void {
    this.tabRole = 'follower';
    this.hub = null;
    this.forwarder = forwarder;
    // A still-running resumeLeaderDuties self-cancels on its `tabRole` guard;
    // drop the handle so a later re-promotion starts a fresh drain.
    this.leaderDutiesInFlight = null;
    void this.killRefLiveQuery();
    forwarder.onLeaderMessage = (msg) => {
      switch (msg.type) {
        case 'ingest-relay':
          this.applyRelayedIngest(msg.tuples);
          break;
        case 'mutation-settled':
          // The leader pushed a write and deleted its outbox row from the
          // shared store. Without this the row would leave this tab's render
          // set (it is in neither membership nor pending writes) until the
          // relayed `_00_list_ref` event lands: the blink the leader itself
          // is already protected from by `handleMutationSettled`.
          this.dataModule.noteWriteSettled(msg.recordId, msg.eventType);
          break;
        case 'list-ref-change':
          void this.applyRelayedListRefChange(msg).catch((err) => {
            this.logger.error(
              { err, Category: 'sp00ky-client::Sp00kySync::relay' },
              'Relayed list_ref change failed'
            );
          });
          break;
        case 'mutation-rolled-back':
          this.events.emit(SyncEventTypes.MutationRolledBack, {
            eventType: msg.eventType,
            recordId: msg.recordId,
            error: msg.error,
          });
          break;
        default:
          // db-ready is consumed by the coordinator's attach handshake before
          // the sync handler is installed.
          break;
      }
    };
  }

  /**
   * A pending mutation was discarded because it can never be sent.
   *
   * This is a lost write, so it must not stay invisible. Every failure in this
   * chain used to be a `logger.error` an app running `logLevel: 'fatal'` never
   * shows, which is how an outbox could sit undrained for hours with the UI
   * reporting nothing. Surfaces as a rollback event (the mutation will never
   * apply, which is what a subscriber needs to know) and degrades sync health.
   */
  private onMutationDropped(dropped: {
    mutationId: string;
    recordId?: string;
    mutationType?: string;
    reason: string;
  }): void {
    this.logger.error(
      { ...dropped, Category: 'sp00ky-client::Sp00kySync::onMutationDropped' },
      'Dropped a pending mutation that can never be sent'
    );
    this.recordSyncOutcome(false, new Error(`dropped mutation: ${dropped.reason}`));
    this.events.emit(SyncEventTypes.MutationRolledBack, {
      eventType: (dropped.mutationType as 'create' | 'update' | 'delete') ?? 'update',
      recordId: dropped.recordId ?? dropped.mutationId,
      error: `dropped: ${dropped.reason}`,
    });
  }

  /** A forwarded outbox row from a follower: load + drain it. Idempotent. */
  public async enqueueForwardedMutation(mutationId: string): Promise<void> {
    if (this.tabRole !== 'leader') return;
    await this.upQueue.enqueueFromDatabase(mutationId);
  }

  /**
   * Tuples another tab already committed to the shared store: feed them to
   * THIS tab's circuit (no local write). A DELETE additionally forces a
   * re-materialize of the table's queries, exactly as the writing tab does
   * for itself, because the SSP may not emit a view update for it.
   */
  private applyRelayedIngest(tuples: IngestTuple[]): void {
    this.cache.applyRelayedIngest(tuples);
    const deletedTables = new Set<string>();
    for (const t of tuples) if (t.op === 'DELETE') deletedTables.add(t.table);
    for (const table of deletedTables) {
      void this.dataModule.notifyTableQueries(table).catch((err) => {
        this.logger.warn(
          { err, table, Category: 'sp00ky-client::Sp00kySync::applyRelayedIngest' },
          'Re-materialize after relayed delete failed'
        );
      });
    }
  }

  /** A relayed `_00_list_ref` LIVE event: resolve against THIS tab's queries
   *  and run the exact same handling the LIVE subscription would have. */
  private async applyRelayedListRefChange(msg: {
    action: 'CREATE' | 'UPDATE' | 'DELETE';
    queryId: string;
    recordId: string;
    version: number;
    parent: boolean;
  }): Promise<void> {
    const queryId = parseRecordIdString(msg.queryId);
    // Foreign query (another tab's session-salted hash): not ours, ignore.
    if (!this.dataModule.getQueryById(queryId)) return;
    const recordId = parseRecordIdString(msg.recordId);
    if (msg.parent) {
      await this.handleRemoteSubqueryChange(msg.action, queryId, recordId, msg.version);
    } else {
      await this.handleRemoteListRefChange(msg.action, queryId, recordId, msg.version);
    }
  }

  /** One immediate poll cycle (failover convergence). */
  public async forcePollRound(): Promise<void> {
    this.listRefIdleStreak = 0;
    await this.pollListRefForActiveQueries().catch(() => false);
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
        const startedAt = Date.now();
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
          // it when a LIVE event lands.) A settled write still waiting for its
          // membership also pins the fast cadence: the row is on borrowed time
          // (the settled-write grace), so the edge must be found on the first
          // poll after it lands, not after the backoff has grown to the cap.
          const awaitingMembership = this.dataModule.hasSettledWritesPending();
          this.listRefIdleStreak = changed || awaitingMembership ? 0 : this.listRefIdleStreak + 1;
          // Never let the poll take more than half the wall clock: a cycle that
          // took longer than the cadence (many queries, slow link) waits at
          // least its own duration before the next one, so it cannot occupy
          // the remote back-to-back the way the old per-query poll did.
          const cycleMs = Date.now() - startedAt;
          const next = Math.max(
            listRefPollDelayMs({
              idleStreak: this.listRefIdleStreak,
              baseIntervalMs: this.refSyncIntervalMs,
            }),
            cycleMs
          );
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
   * One poll cycle: refetch `_00_list_ref` for the active queries that are due,
   * one round trip per chunk (see `planListRefPollChunks`). Returns whether ANY
   * query's remoteArray actually changed — the scheduler uses this to drive the
   * adaptive idle backoff.
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
    const now = Date.now();
    // Forget queries that went away so the map cannot grow with the session.
    const active = new Set(hashes);
    for (const hash of this.listRefPolledAt.keys()) {
      if (!active.has(hash)) this.listRefPolledAt.delete(hash);
    }
    const chunks = planListRefPollChunks(
      hashes.map((hash) => ({
        hash,
        rows: this.dataModule.getQueryByHash(hash)?.config.remoteArray?.length ?? 0,
        lastPolledAt: this.listRefPolledAt.get(hash) ?? 0,
      })),
      { now }
    );
    let anyChanged = false;
    // `reached` = the server answered at least once this cycle (a success, or an
    // *application* error, which still proves reachability). `firstNetworkErr`
    // holds the first network-classified failure. A cycle that only produced
    // network errors reports the outcome as a down round; a mixed/app cycle counts
    // as reached; an all-application cycle reports nothing (that's a query-shape
    // fault owned by the registration path, not a reachability signal).
    let reached = false;
    let firstNetworkErr: unknown;
    for (const chunk of chunks) {
      let snapshots: Map<string, ListRefSnapshot>;
      try {
        snapshots = await this.fetchListRefSnapshots(chunk);
        reached = true;
      } catch (err) {
        if (classifySyncError(err) === 'network') {
          if (firstNetworkErr === undefined) firstNetworkErr = err;
        } else {
          reached = true;
        }
        this.logger.debug(
          {
            err: (err as Error)?.message ?? err,
            hashes: chunk,
            Category: 'sp00ky-client::Sp00kySync::pollListRefForActiveQueries',
          },
          'list_ref poll round trip failed'
        );
        continue;
      }
      for (const hash of chunk) {
        const snapshot = snapshots.get(hash);
        if (!snapshot) continue;
        this.listRefPolledAt.set(hash, now);
        try {
          if (await this.applyListRefSnapshot(hash, snapshot)) anyChanged = true;
        } catch (err) {
          this.logger.debug(
            {
              err: (err as Error)?.message ?? err,
              hash,
              Category: 'sp00ky-client::Sp00kySync::pollListRefForActiveQueries',
            },
            'Per-query list_ref poll apply failed'
          );
        }
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
   * The server's current `_00_list_ref` state for a chunk of queries in ONE
   * round trip: primary edges, subquery-child edges and `_00_query.rowCount`.
   * Every hash whose query still exists gets an entry (empty arrays and
   * `rowCount: null` when nothing was readable), so a caller can tell "no
   * edges" from "not asked". A hash is left out only when the select did not
   * come back as an array at all — the old per-query code skipped the apply in
   * that case too, since a non-array is not a membership set.
   *
   * Guarded against a batch that silently matches nothing: if the select
   * returned no edge at all while the client holds edges for a query the
   * server still reports rows for, the batch is not trusted and the chunk is
   * re-read one query at a time with the single-query select. An empty set
   * taken at face value would be recorded as the removal of every row.
   */
  private async fetchListRefSnapshots(hashes: string[]): Promise<Map<string, ListRefSnapshot>> {
    const listRefTbl = this.listRefTable();
    const ids: RecordId<string>[] = [];
    const hashById = new Map<string, string>();
    const out = new Map<string, ListRefSnapshot>();
    for (const hash of hashes) {
      const queryState = this.dataModule.getQueryByHash(hash);
      if (!queryState) continue;
      ids.push(queryState.config.id);
      hashById.set(encodeRecordId(queryState.config.id), hash);
    }
    if (ids.length === 0) return out;
    if (ids.length === 1) {
      const hash = hashById.get(encodeRecordId(ids[0]))!;
      const single = await this.fetchListRefSnapshot(ids[0]);
      if (single) out.set(hash, single);
      return out;
    }
    const [edges, counts] = await this.remote.query<
      [ListRefEdgeRow[] | null, ({ id?: RecordId<string>; rowCount?: number | null } | null)[] | null]
    >(`${buildListRefBatchSelect(listRefTbl)};\n${buildQueryRowCountBatchSelect()}`, { ins: ids });
    if (!Array.isArray(edges)) return out;
    for (const hash of hashById.values()) {
      out.set(hash, { primary: [], subquery: [], rowCount: null });
    }
    for (const row of edges) {
      const hash = hashById.get(encodeRecordId(row.in));
      if (!hash) continue;
      const snapshot = out.get(hash)!;
      const pair: [string, number] = [encodeRecordId(row.out), row.version];
      if (row.parent == null) snapshot.primary.push(pair);
      else snapshot.subquery.push(pair);
    }
    for (const count of Array.isArray(counts) ? counts : []) {
      if (!count || !count.id) continue;
      const hash = hashById.get(encodeRecordId(count.id));
      if (!hash) continue;
      out.get(hash)!.rowCount = typeof count.rowCount === 'number' ? count.rowCount : null;
    }
    const suspect =
      edges.length === 0 &&
      Array.from(out.entries()).some(([hash, snapshot]) => {
        const held = this.dataModule.getQueryByHash(hash)?.config.remoteArray?.length ?? 0;
        return held > 0 && (snapshot.rowCount ?? 0) > 0;
      });
    if (suspect) {
      this.logger.warn(
        { hashes, Category: 'sp00ky-client::Sp00kySync::fetchListRefSnapshots' },
        'Batched list_ref select returned no edges for queries that hold rows; re-reading one query at a time'
      );
      for (const hash of Array.from(out.keys())) {
        const queryState = this.dataModule.getQueryByHash(hash);
        if (!queryState) {
          out.delete(hash);
          continue;
        }
        const single = await this.fetchListRefSnapshot(queryState.config.id);
        if (single) out.set(hash, single);
        else out.delete(hash);
      }
    }
    return out;
  }

  /**
   * Single-query form of {@link fetchListRefSnapshots}: the selects the
   * registration path has always used, in one statement batch. `null` when
   * the primary select did not come back as an array.
   */
  private async fetchListRefSnapshot(id: RecordId<string>): Promise<ListRefSnapshot | null> {
    const listRefTbl = this.listRefTable();
    const [items, serverRowCount, children] = await this.remote.query<
      [
        { out: RecordId<string>; version: number }[] | null,
        number | null,
        { out: RecordId<string>; version: number }[] | null,
      ]
    >(
      `${buildListRefSelect(listRefTbl)};\n${buildQueryRowCountSelect()};\n${buildSubqueryListRefSelect(listRefTbl)}`,
      { in: id }
    );
    if (!Array.isArray(items)) return null;
    const toPairs = (rows: { out: RecordId<string>; version: number }[] | null): RecordVersionArray =>
      Array.isArray(rows) ? rows.map((item) => [encodeRecordId(item.out), item.version]) : [];
    return {
      primary: toPairs(items),
      subquery: toPairs(children),
      rowCount: typeof serverRowCount === 'number' ? serverRowCount : null,
    };
  }

  /**
   * Land one poll snapshot for `queryHash`: diff the server's edges against the
   * cached `remoteArray`, sync any added/updated rows through the SyncEngine,
   * persist the new remoteArray and converge the subquery children. This is
   * the same shape `createRemoteQuery` does for its initial fetch and what
   * `handleRemoteListRefChange` does per-LIVE-event — we reuse it on a timer
   * as a fallback for missed LIVE notifications. Returns whether membership
   * changed.
   */
  private async applyListRefSnapshot(
    queryHash: string,
    snapshot: ListRefSnapshot
  ): Promise<boolean> {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return false;
    const fresh = snapshot.primary;
    const serverRowCount = snapshot.rowCount;
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
      await this.dataModule.updateQueryRemoteArray(queryHash, fresh, { serverRowCount });
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
        {
          err: (err as Error)?.message ?? err,
          queryHash,
          Category: 'sp00ky-client::Sp00kySync::applyListRefSnapshot',
        },
        'syncQuery failed during poll'
      );
    }
    // Membership moved but a row may have needed no fetch (this client wrote
    // it, so the engine already holds it at the published version): nothing
    // then re-materializes the query. Ask for one; it is a no-op behind a
    // stream update that a fetch above already queued.
    if (changed) this.dataModule.scheduleRematerialize(queryHash);
    // Cross-session fallback for `.related()` child rows: the LIVE-permission
    // gap can drop child-edge notifications, so converge their bodies on the
    // poll too (idempotent — no-op when nothing changed).
    await this.applySubqueryChildren(queryHash, snapshot.subquery).catch((err) => {
      this.logger.info(
        {
          err: (err as Error)?.message ?? err,
          queryHash,
          Category: 'sp00ky-client::Sp00kySync::applyListRefSnapshot',
        },
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
          {
            err: (err as Error)?.message ?? err,
            queryHash,
            Category: 'sp00ky-client::Sp00kySync::applyListRefSnapshot',
          },
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
      try {
        this.liveQueryUnsubscribe();
      } catch {
        /* ignore */
      }
      this.liveQueryUnsubscribe = null;
    }
    if (this.currentLiveQueryUuid !== null) {
      // A LIVE subscription is scoped to its WebSocket session, so after a
      // reconnect the server-side one is already gone and there is nothing to
      // KILL. Sending it anyway either fails (no connection) or races the fresh
      // socket's readiness while holding up the restart behind it. Local
      // bookkeeping is cleared either way.
      if (this.remote.getStatus() === 'connected') {
        try {
          await this.remote.query('KILL $u', { u: this.currentLiveQueryUuid });
        } catch (err) {
          this.logger.debug(
            { err, Category: 'sp00ky-client::Sp00kySync::killRefLiveQuery' },
            'Prior LIVE KILL failed; continuing'
          );
        }
      }
      this.currentLiveQueryUuid = null;
    }
  }

  private async restartRefLiveQuery(): Promise<void> {
    await this.killRefLiveQuery();
    await this.startRefLiveQueries();
  }

  /**
   * Drop local LIVE bookkeeping without issuing a `KILL`.
   *
   * Called when the socket dies. The server-side subscription is scoped to that
   * WebSocket session and died with it, so there is nothing left to kill — and
   * by the time the reconnect handler runs, the client reports `connected`
   * again, which would otherwise send a `KILL` for a stale uuid on the *new*
   * session and hold up the restart queued behind it.
   */
  private invalidateRefLiveQuery(): void {
    if (this.liveQueryUnsubscribe) {
      try {
        this.liveQueryUnsubscribe();
      } catch {
        /* ignore */
      }
      this.liveQueryUnsubscribe = null;
    }
    this.currentLiveQueryUuid = null;
  }

  // Only the connect that follows a prior drop counts as a reconnect; the
  // initial connect after init() must not trigger a refetch storm.
  //
  // Both drop events have to be watched. The SDK publishes `disconnected` ONLY
  // when it has given up entirely (attempts exhausted, or the engine
  // terminated); an ordinary recovered drop goes `error` -> `reconnecting` ->
  // `connected` and never touches `disconnected`. Listening for `disconnected`
  // alone therefore misses every successful reconnect — the exact case this
  // handler exists for — and leaves the dead server-side LIVE in place with the
  // poll as the only sync path.
  private subscribeToReconnect() {
    const client = this.remote.getClient();
    client.subscribe('disconnected', () => {
      this.needsResubscribe = true;
      this.invalidateRefLiveQuery();
      this.logger.info(
        { Category: 'sp00ky-client::Sp00kySync::onDisconnect' },
        'Remote disconnected'
      );
    });
    client.subscribe('reconnecting', () => {
      this.needsResubscribe = true;
      this.invalidateRefLiveQuery();
      this.logger.info(
        { Category: 'sp00ky-client::Sp00kySync::onReconnecting' },
        'Remote socket dropped; awaiting reconnect'
      );
    });
    client.subscribe('connected', () => {
      if (!this.needsResubscribe) return;
      this.needsResubscribe = false;
      // A flapping socket produces reconnecting -> connected repeatedly, and
      // each cycle used to re-register EVERY active query (a busy app has
      // dozens). That is the "everything reloads about a second after a blip"
      // symptom: the SDK's retryDelay is 1s, so the refetch lands right after
      // the drop the user never saw. Collapse bursts into one refetch.
      const sinceLast = Date.now() - this.lastReconnectRefetchAt;
      if (sinceLast < Sp00kySync.RECONNECT_REFETCH_COOLDOWN_MS) {
        this.logger.debug(
          { sinceLast, Category: 'sp00ky-client::Sp00kySync::onReconnect' },
          'Reconnected again within the cooldown; skipping duplicate refetch'
        );
        return;
      }
      this.lastReconnectRefetchAt = Date.now();
      void this.refetchAfterReconnect();
    });
  }

  /**
   * Whether the REMOTE SESSION currently carries `$auth.id`.
   *
   * Not the same question as `currentUserId`, and that gap is the whole point:
   * `currentUserId` is this client's own record of who signed in and survives a
   * socket drop untouched, while `$auth` lives on the WebSocket session and has
   * to be re-applied after every reconnect. Registering in the window between
   * the two is silently destructive, because `fn::query::register` sends
   * `<string>($auth.id OR '')` and the SSP stores that value write-once: the
   * view's edges then route to the global `_00_list_ref` stamped `auth_id = ''`,
   * which that table's own permission rule (`auth_id = $auth.id`) makes
   * unreadable to the very user who registered it.
   *
   * Signed-out clients answer `true`: `''` is the honest identity there, not a
   * race.
   */
  private async remoteAuthEstablished(): Promise<boolean> {
    if (!this.currentUserId) return true;
    try {
      const result = await this.remote.query<[string]>("RETURN <string>($auth.id OR '')");
      const authId = Array.isArray(result) ? result[0] : undefined;
      return typeof authId === 'string' && authId.length > 0;
    } catch (err) {
      this.logger.debug(
        { err, Category: 'sp00ky-client::Sp00kySync::remoteAuthEstablished' },
        'Auth probe failed; treating the session as not yet authenticated'
      );
      return false;
    }
  }

  /**
   * Wait for the reconnected session to carry an identity again, then
   * re-register every active query and re-bind LIVE.
   *
   * Giving up without registering is deliberately better than registering
   * anyway: a registration made with no `$auth.id` produces a view its own
   * owner cannot read, and it is write-once, so it stays that way. Skipping
   * leaves the query unregistered and visibly loading, which the next
   * reconnect or heartbeat retries.
   */
  private async refetchAfterReconnect(): Promise<void> {
    for (let attempt = 1; attempt <= Sp00kySync.AUTH_READY_MAX_ATTEMPTS; attempt++) {
      if (await this.remoteAuthEstablished()) break;
      if (attempt === Sp00kySync.AUTH_READY_MAX_ATTEMPTS) {
        this.logger.warn(
          {
            attempts: attempt,
            Category: 'sp00ky-client::Sp00kySync::onReconnect',
          },
          'Reconnected but the session still carries no $auth.id; not re-registering (a registration now would stamp every view with an empty identity)'
        );
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, Sp00kySync.AUTH_READY_RETRY_MS));
    }

    const hashes = this.dataModule.getActiveQueryHashes();
    this.logger.info(
      { queries: hashes.length, Category: 'sp00ky-client::Sp00kySync::onReconnect' },
      'Remote reconnected, refetching active queries'
    );
    for (const hash of hashes) {
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
  }

  private async startRefLiveQueries() {
    // Shared-tabs follower: exactly one LIVE per user exists, on the leader;
    // its events reach this tab through the relay.
    if (this.tabRole === 'follower') return;
    const tableName = this.listRefTable();
    this.logger.debug(
      { tableName, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
      'Starting ref live queries'
    );

    const [queryUuid] = await this.remote.query<[Uuid]>(`LIVE SELECT * FROM ${tableName}`);
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

    // Shared-tabs leader: the list_ref table is USER-scoped, so this LIVE also
    // carries events for FOLLOWER tabs' queries (their own session-salted
    // hashes). Relay every primary event; each follower resolves the queryId
    // against its own DataModule and ignores foreign ones. Then continue with
    // this tab's own handling below.
    this.hub?.broadcast({
      type: 'list-ref-change',
      action,
      queryId: encodeRecordId(queryId),
      recordId: encodeRecordId(recordId),
      version,
      parent: false,
    });

    // NOTE: DELETE is handled like CREATE/UPDATE below. When another window (or
    // this one) deletes a record, the server's SSP removes it from `_00_list_ref`
    // and the LIVE subscription delivers a DELETE here — `createDiffFromDbOp`
    // turns it into a `removed: [recordId]` diff so the row drops from the window
    // in realtime. (It was previously ignored, so other windows only caught up on
    // reload / the slow poll.)
    const existing = this.dataModule.getQueryById(queryId);

    if (!existing) {
      // With a hub attached, an unknown query is the NORMAL case (it belongs
      // to a follower tab); without one it still warrants the warning.
      if (!this.hub) {
        this.logger.warn(
          {
            queryId: queryId.toString(),
            Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
          },
          'Received remote update for unknown local query'
        );
      }
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

    // Apply the event to `remoteArray` — the authoritative membership rows are
    // now rendered FROM. Only registration and the poll used to write it, so a
    // LIVE removal left the departed id in the list (in memory and persisted)
    // until the next poll tick, which is up to 5s of showing a deleted row. This
    // also persists the durable `_00_window` mirror, so the removal survives a
    // reload with no network.
    //
    // Derived from the raw action, NOT from `diff`: `createDiffFromDbOp` is
    // empty when the circuit already holds the row at this version, which is
    // every tab that ingested the write optimistically (its own, or one
    // relayed from another tab). The fetch is rightly skipped then, but the
    // membership still has to be recorded, or the row lives on the
    // settled-write grace alone until the poll catches it.
    if (existing.config.membershipKnown) {
      const membershipDiff: RecordVersionDiff =
        action === 'DELETE'
          ? { added: [], updated: [], removed: [recordId] }
          : { added: [{ id: recordId, version }], updated: [], removed: [] };
      const next = applyRecordVersionDiff(existing.config.remoteArray ?? [], membershipDiff);
      if (!recordVersionArraysEqual(next, existing.config.remoteArray ?? [])) {
        await this.dataModule.updateQueryRemoteArray(hash, next);
      }
    }

    await this.runSyncForQuery(hash, diff);

    // A removal-only diff sets `fetching` false in `runSyncForQuery`, so it gets
    // no `flushPendingStreamUpdate`/`endFetching` re-render — and a removal needs
    // no record fetch to trigger one either. Force it, mirroring what the poll
    // path already does for its own removals (`refetchListRefForQuery`).
    if (diff.removed.length > 0 && diff.added.length === 0 && diff.updated.length === 0) {
      await this.dataModule.notifyQuerySynced(hash);
    } else if (diff.added.length === 0 && diff.updated.length === 0) {
      // Empty diff: the circuit already holds the row at this version (this
      // tab wrote it), so no fetch and no stream update - but the membership
      // recorded above is new, and the subscribers have to hear about it.
      this.dataModule.scheduleRematerialize(hash);
    }
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

    // Relay child-edge events too (see handleRemoteListRefChange).
    this.hub?.broadcast({
      type: 'list-ref-change',
      action,
      queryId: encodeRecordId(queryId),
      recordId: encodeRecordId(childId),
      version,
      parent: true,
    });

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

  /**
   * Bound a mutation push so it always settles.
   *
   * `SyncScheduler.syncUp` early-returns while `isSyncingUp` is true, and that
   * flag only clears in the `finally` of the drain loop. A push whose RPC never
   * settles (socket dropped mid-flight, response lost) therefore wedges the
   * up-queue for the rest of the session: no retry, no error, no further
   * mutation ever sent. A timeout turns that into an ordinary network failure,
   * which `UpQueue.next` re-queues for the next trigger. The message deliberately
   * contains "timed out" so `classifySyncError` treats it as `network` and
   * retries rather than rolling the mutation back.
   */
  private withPushTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
    return withTimeout(
      promise,
      this.pushTimeoutMs,
      `Mutation push timed out after ${this.pushTimeoutMs}ms (${label})`
    );
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
        await this.withPushTimeout(
          this.remote.query(query, {
            id: event.record_id,
            ...prefixedParams,
          }),
          'create'
        );
        break;
      }
      case 'update':
        await this.withPushTimeout(
          this.remote.query(`UPDATE $id MERGE $data`, {
            id: event.record_id,
            data: event.data,
          }),
          'update'
        );
        break;
      case 'delete':
        await this.withPushTimeout(
          this.remote.query(`DELETE $id`, {
            id: event.record_id,
          }),
          'delete'
        );
        break;
      default:
        this.logger.error(
          { event, Category: 'sp00ky-client::Sp00kySync::processUpEvent' },
          'processUpEvent unknown event type'
        );
        return;
    }
  }

  /**
   * A mutation the server accepted, reported once its outbox row is gone.
   *
   * Keeps the written row in the render set until its membership arrives.
   * Without this the row is briefly in neither term of
   * `(membership ∪ pendingWrites) − pendingDeletes` — the outbox delete is
   * tied to the push, while membership waits on the SSP ingesting the row,
   * materializing the view, writing the `_00_list_ref` edge and this client
   * reading it back. The writer therefore watched its own comment appear,
   * vanish, and return, while every other client showed it throughout.
   */
  private handleMutationSettled(event: UpEvent): void {
    const recordId = encodeRecordId(event.record_id);
    this.dataModule.noteWriteSettled(recordId, event.type);
    // Shared-tabs: the outbox row just left the SHARED store, so every
    // follower rendering the row as a pending write has the same gap. All of
    // them, not just the owner: any tab whose query matched the row was
    // showing it through `pendingWrites`.
    this.hub?.broadcast({
      type: 'mutation-settled',
      mutationId: encodeRecordId(event.mutation_id),
      recordId,
      eventType: event.type,
    });
  }

  private async handleRollback(event: UpEvent, error: Error): Promise<void> {
    const recordId = encodeRecordId(event.record_id);
    const tableName =
      event.type === 'create' && event.tableName ? event.tableName : extractTablePart(recordId);

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

    // Shared-tabs: the store rollback above already propagated to every tab
    // via the ingest relay; additionally deliver the EVENT to the tab that
    // owns the mutation so its UI (toasts, subscribeToRollbacks) fires there.
    const mutationId = encodeRecordId(event.mutation_id);
    const owner = mutationOwnerTabId(mutationId);
    if (this.hub && owner && this.tabId && owner !== this.tabId) {
      this.hub.sendTo(owner, {
        type: 'mutation-rolled-back',
        mutationId,
        recordId,
        eventType: event.type,
        error: error.message,
      });
    }
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug(
      { event, Category: 'sp00ky-client::Sp00kySync::processDownEvent' },
      'Processing down event'
    );
    // Bounded for the same reason a push is (see withPushTimeout): an RPC that
    // never settles would otherwise hold its slot in the concurrent down drain
    // — and, before that drain existed, the WHOLE queue — for the rest of the
    // session, with no retry, no error, and every dependent `useQuery` stuck
    // loading. "timed out" in the message keeps `classifySyncError` treating it
    // as a network failure, so `DownQueue.run` re-heads it for the next pass.
    return this.withDownTimeout(this.runDownEvent(event), `${event.type} ${event.payload.hash}`);
  }

  private withDownTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
    return withTimeout(
      promise,
      this.downTimeoutMs,
      `Down event timed out after ${this.downTimeoutMs}ms (${label})`
    );
  }

  private async runDownEvent(event: DownEvent): Promise<void> {
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
    // The diff was computed against whatever the circuit held at call time; if
    // the boot prime is still filling it, wait and recompute from the primed
    // `localArray` so the delta is real rather than "everything".
    const prime = this.primeGate();
    if (prime !== this.settledPrime) {
      await prime;
      this.settledPrime = prime;
      const fresh = this.dataModule.getQueryByHash(hash);
      if (!fresh) return;
      const recomputed = new ArraySyncer(fresh.config.localArray, fresh.config.remoteArray).nextSet();
      if (!recomputed) return;
      diff = recomputed;
    }
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
        //
        // A fetch that changed a row already in membership can come with no
        // stream update at all: the body lands in the local store, but the
        // circuit sees the same id-set and stays silent (measured: a peer's
        // UPDATE to a `conversation` row reached the store at `_00_rv` 8 while
        // the query kept rendering version 2 indefinitely). Nothing else
        // re-reads the query then, so ask for a synthetic re-materialize and
        // land it here too. It is one local select, and a no-op when a real
        // update was pending, which the first flush already processed.
        try {
          const landed = await this.dataModule.flushPendingStreamUpdate(hash);
          if (!landed) {
            this.dataModule.scheduleRematerialize(hash);
            await this.dataModule.flushPendingStreamUpdate(hash);
          }
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
    // Single implementation, shared with the render path: `materializeRecords`
    // subtracts the same set so a row whose DELETE is still in the outbox is
    // neither re-fetched here nor rendered there.
    return (await this.dataModule.getPendingRecordIds()).deletes;
  }

  /**
   * Enqueues a list of mutations (up events) to be sent to the remote.
   * @param mutations Array of UpEvents (create/update/delete) to enqueue.
   */
  public async enqueueMutation(mutations: UpEvent[]) {
    // Follower: the outbox rows are already committed in the SHARED store (the
    // mutation tx went through the leader's worker); only the leader drains,
    // so hand over the ids instead of queueing locally. A notify lost in a
    // failover window is covered by the new leader's loadFromDatabase.
    if (this.tabRole === 'follower') {
      for (const m of mutations) {
        this.forwarder?.mutationEnqueued(encodeRecordId(m.mutation_id));
      }
      return;
    }
    this.scheduler.enqueueMutation(mutations);
  }

  /**
   * Best-effort release of THIS tab's views as the page goes away.
   *
   * Without it a closed tab's views stay materialized on the SSP for a full
   * TTL (10 minutes by default), because nothing else tears them down:
   * `releaseQueriesEagerly` is off, so the TTL sweep is the only reclaim path.
   * On a busy tenant that is a generation of live views per reload, each still
   * being stepped by every ingest, for ten minutes after the last human left.
   *
   * This is NOT `releaseQueriesEagerly` re-enabled. That released views
   * mid-session on a viewport change and tore the window out from under a live
   * page (see `cleanupQuery`); this runs only when the page itself is going
   * away and nothing is left to render.
   *
   * Safe against other tabs by construction: `fn::query::unsubscribe` drops
   * only this session from `subscribers` and deletes the row solely when it
   * was the last one, so a second tab of the same user keeps its view.
   *
   * One statement, not one per id: an unload handler gets a few milliseconds,
   * and N round trips would not survive it. Fire-and-forget — if the frame
   * does not make it out, the TTL sweep still reclaims exactly as before, so
   * the worst case is today's behaviour.
   */
  releaseViewsOnUnload(): void {
    let ids: unknown[];
    try {
      ids = this.dataModule
        .getActiveQueryHashes()
        .map((hash) => this.dataModule.getQueryByHash(hash)?.config.id)
        .filter((id): id is NonNullable<typeof id> => id != null);
    } catch {
      return;
    }
    if (ids.length === 0) return;

    // Over HTTP with `keepalive`, NOT the live socket. A WebSocket send during
    // `pagehide` is not guaranteed to flush — the browser may tear the socket
    // down first and the frame is lost. Measured on staging: releasing over the
    // socket reached the server zero times; `fetch`/`keepalive` is the only
    // primitive the platform promises to finish after the page is gone.
    //
    // Ids are inlined rather than bound because this is a bare statement, not
    // an RPC call with a params channel. They are `_00_query:<sha256>` record
    // ids the client itself derived, so the only interpolation is a hex digest.
    const list = ids
      .map((id) => String(id))
      .filter((id) => /^_00_query:[0-9a-f]{64}$/.test(id))
      .join(', ');
    if (!list) return;

    this.remote.beaconSql(
      `FOR $id IN [${list}] { LET $_released = fn::query::unsubscribe($id); };`
    );
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
    // `rowCount` rides along: it is written by the SSP in the same statement
    // that registers the view, BEFORE the edges are flushed, so it is the only
    // way to tell "this query is empty" from "its edges have not landed yet".
    const [items, serverRowCount] = await this.remote.query<
      [{ out: RecordId<string>; version: number }[], number | null]
    >(`${buildListRefSelect(listRefTbl)};\n${buildQueryRowCountSelect()}`, {
      in: queryState.config.id,
    });

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
      await this.dataModule.updateQueryRemoteArray(queryHash, array, { serverRowCount });
    }

    // Pull the bodies of any `.related()` subquery children into the local
    // cache. The primary fetch above (`parent IS NONE`) tracks only window
    // rows, so without this a cold-reload re-materialization of the
    // correlated surql finds no child rows and related fields come back
    // empty. Best-effort: never fail registration over it.
    await this.syncSubqueryChildren(queryHash).catch((err) => {
      this.logger.info(
        {
          err: (err as Error)?.message ?? err,
          queryHash,
          Category: 'sp00ky-client::Sp00kySync::createRemoteQuery',
        },
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
    await this.applySubqueryChildren(queryHash, fresh);
  }

  /**
   * Converge a `.related()` query's child bodies onto `fresh` (the server's
   * current child edges). Shared by the registration path above and the poll,
   * which reads the child edges in its batched round trip. See
   * {@link syncSubqueryChildren} for the deletion-safety rules.
   */
  private async applySubqueryChildren(queryHash: string, fresh: RecordVersionArray): Promise<void> {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return;
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
    // `fn::query::heartbeat` is an `UPDATE $id SET ...`. On a record that no
    // longer exists that matches nothing and returns an empty array — it does
    // NOT recreate the row. So an unchecked heartbeat is indistinguishable from
    // a successful one, and a client whose row was reclaimed keeps beating
    // against nothing forever: no membership, no edges, no re-registration.
    // The page renders as if the data were deleted ("Game not found").
    //
    // A live query's row is reclaimed more easily than it looks. The sweep
    // expires on `lastActiveAt + ttl`, and this heartbeat runs on a timer that
    // browsers throttle hard in background tabs — so a second window left idle
    // past its TTL is the ordinary way to get here, not an edge case. Until
    // canary.194 the sweep could not actually remove the in-memory view (it
    // looked it up under the other of the two query-id spellings), which masked
    // this: the view survived its own row. Now reclamation is real, so the
    // client has to notice and rebuild.
    const result = await this.remote.query('fn::query::heartbeat($id)', {
      id: queryState.config.id,
    });
    const updated = Array.isArray(result) ? result[0] : undefined;
    const rowGone = Array.isArray(updated) && updated.length === 0;
    if (!rowGone) return;

    this.logger.warn(
      {
        queryHash,
        id: String(queryState.config.id),
        Category: 'sp00ky-client::Sp00kySync::heartbeatQuery',
      },
      'Query row was reclaimed while still in use; re-registering'
    );
    // Re-register rather than recreate the row here: the row alone is useless
    // without the SSP view behind it, and only registration rebuilds the view,
    // republishes `_00_list_ref` and writes `rowCount`.
    this.enqueueDownEvent({ type: 'register', payload: { hash: queryHash } });
  }

  // Eager teardown of a deregistered query's remote `_00_query` view (opt-in,
  // e.g. a viewport-windowed list cancelling an off-screen window). Query ids
  // are a deterministic hash of (surql+params), so a release racing a
  // scroll-back re-register (same id) could nuke a freshly-recreated view —
  // hence two guards: abort if a subscriber reappeared BEFORE the release;
  // re-register if one reappears DURING the release's network await. Tolerant
  // of a missing/already-gone query (no throw).
  //
  // Releases via `fn::query::unsubscribe` rather than deleting the row
  // outright. A `_00_query` row can be shared by several sessions of the same
  // user, so a bare `DELETE` would tear the view — and every `_00_list_ref`
  // edge hanging off it — out from under other live tabs. The function drops
  // only this session from `subscribers` and deletes the row when it was the
  // last one. (The old `DELETE $id` was harmless in practice only because the
  // table granted no delete permission and it silently affected zero rows.)
  private async cleanupQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return; // already torn down / never registered

    // Re-subscribed before the queued cleanup ran → keep everything as-is.
    if (this.dataModule.hasSubscribers(queryHash)) return;

    // EAGER REMOTE RELEASE IS DISABLED. Deliberate, and not a leak: the TTL
    // sweep reclaims the row and its edges on `lastActiveAt + ttl`, which is the
    // ONLY reclamation that has ever actually run in production.
    //
    // Until canary.190 `_00_query` granted no delete permission, so the bare
    // `DELETE $id` this used to issue affected zero rows. .190 granted delete
    // and .191 wired `fn::query::unsubscribe`, which made teardown real for the
    // first time -- and the guards above are best-effort by construction
    // (`hasSubscribers` can be momentarily false during a rebind or a windowed
    // list re-flow). Every misfire that had been silently inert for months
    // became a live delete of the row AND every `_00_list_ref` edge on it.
    //
    // That matches a report of chat suddenly rendering raw record ids instead
    // of users, with the message list re-flowing underneath. Server state was
    // measured intact at the time (`rowCount` equalled the actual edge count on
    // every row), so the damage is on the client side of a teardown, not in the
    // materialization.
    //
    // Re-enable only together with a repair path that can re-fetch subquery
    // child bodies whose `subqueryRemoteArray` entry claims they are already
    // synced -- otherwise a torn-down-and-recreated view never restores the
    // related records it dropped, because the idempotence check skips them.
    if (this.releaseQueriesEagerly) {
      await this.remote.query('fn::query::unsubscribe($id)', {
        id: queryState.config.id,
      });

      // Re-subscribed while we awaited the release → re-register. Covers both
      // outcomes: if we were the last subscriber the remote view is gone and
      // this recreates it, and if it survived for other sessions this re-adds
      // us to `subscribers` so our heartbeats keep counting.
      if (this.dataModule.hasSubscribers(queryHash)) {
        this.enqueueDownEvent({ type: 'register', payload: { hash: queryHash } });
        return;
      }
    }

    // No subscribers throughout → safe to free the local view + state.
    this.dataModule.finalizeDeregister(queryHash);
  }
}
