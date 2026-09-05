import { RecordId, Duration } from 'surrealdb';
import type {
  SchemaStructure,
  TableNames,
  BackendNames,
  BackendRoutes,
  RoutePayload,
  QueryPlan,
} from '@spooky-sync/query-builder';
import type { LocalStore } from '../../services/database/index';
import { StaleEpochError } from '../../services/database/index';
import type { CacheModule, RecordWithId, CacheRecord } from '../cache/index';
import type { Logger } from '../../services/logger/index';
import type { StreamUpdate } from '../../services/stream-processor/index';
import type {
  QueryConfig,
  QueryHash,
  QueryState,
  QueryStatus,
  QueryStatusCallback,
  QueryTimeToLive,
  QueryUpdateCallback,
  MutationCallback,
  RecordVersionArray,
  QueryConfigRecord,
  UpdateOptions,
  QueryTimings,
  PhaseStat,
  RegistrationTimings,
  RunOptions,
} from '../../types';
import { MATERIALIZATION_SAMPLE_WINDOW } from '../../types';
import {
  parseRecordIdString,
  extractIdPart,
  encodeRecordId,
  parseDuration,
  withRetry,
  surql,
  parseParams,
  parseQueryParams,
  cleanRecord,
  extractTablePart,
  generateId,
} from '../../utils/index';
import type { CreateEvent, DeleteEvent, UpdateEvent } from '../sync/index';
import type { PushEventOptions } from '../../events/index';
import {
  buildIdSetPlan,
  buildIdSetSurql,
  buildWindowMaterialization,
  buildWindowMaterializationPlan,
} from './window-query';
import { mintMutationId } from './mutation-id';

/**
 * How many consecutive empty `_00_list_ref` reads it takes to believe a query
 * really is empty, before any non-empty set has been seen this session. One
 * read is the registration race (the SSP flushes a view's initial edges
 * asynchronously); the second comes from the poll a cycle later.
 */
const EMPTY_MEMBERSHIP_CONFIRMATIONS = 2;

/** Push a timing sample (ms) into a rolling window, capped at the sample window. */
function pushSample(samples: number[], ms: number): void {
  samples.push(ms);
  if (samples.length > MATERIALIZATION_SAMPLE_WINDOW) samples.shift();
}

/** Build a {lastMs,p50,p90,p99,count} summary from a rolling sample window. */
function phaseStatOf(samples: number[], lastMs: number | null): PhaseStat {
  if (samples.length === 0) {
    return { lastMs, p50: null, p90: null, p99: null, count: 0 };
  }
  const sorted = [...samples].sort((a, b) => a - b);
  const pick = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!;
  return { lastMs, p50: pick(0.5), p90: pick(0.9), p99: pick(0.99), count: samples.length };
}

/**
 * DataModule - Unified query and mutation management
 *
 * Merges the functionality of QueryManager and MutationManager.
 * Uses CacheModule for all storage operations.
 */
/** A `_00_window` row as read back: the id-set and whether the server vouched
 *  for it (which is what allows an empty set to count as known membership). */
export interface DurableMembership {
  ids: RecordVersionArray;
  confirmed: boolean;
}

/**
 * Fraction of a query's TTL after which the client refreshes `lastActiveAt`.
 *
 * Must leave room for at least one retry: the server sweeps a view — and its
 * `_00_list_ref` edges — the moment `lastActiveAt + ttl` passes, and the client
 * is LIVE on those edges, so a swept-but-still-watched view empties the UI.
 */
const TTL_HEARTBEAT_FRACTION = 0.5;

export class DataModule<S extends SchemaStructure> {
  /** Tab identity baked into mutation ids (shared-tabs rollback routing);
   *  undefined in solo mode, where mutation-id falls back to a session id. */
  private tabId: string | undefined;
  private activeQueries: Map<QueryHash, QueryState> = new Map();
  private pendingQueries: Map<QueryHash, Promise<QueryHash>> = new Map();
  private subscriptions: Map<QueryHash, Set<QueryUpdateCallback>> = new Map();
  private statusSubscriptions: Map<QueryHash, Set<QueryStatusCallback>> = new Map();
  private mutationCallbacks: Set<MutationCallback> = new Set();
  private debounceTimers: Map<QueryHash, NodeJS.Timeout> = new Map();
  // The update each debounce timer would process on its trailing edge. Kept in a
  // map (not just the timer closure) so `flushPendingStreamUpdate` can process
  // it early — the sync engine flushes before flipping a query to `idle`, so
  // subscribers never observe idle status with stale (partial-window) rows.
  private pendingStreamUpdates: Map<QueryHash, StreamUpdate> = new Map();
  // Refcount of in-flight fetch cycles per query (registration + concurrent
  // poll/LIVE sync rounds can overlap). Status flips to `fetching` on 0→1 and
  // back to `idle` only on 1→0, so an inner cycle finishing can't emit a
  // premature idle mid-registration.
  private fetchDepth: Map<QueryHash, number> = new Map();
  private logger: Logger;
  /**
   * Optional observer notified whenever a query's fetch status changes.
   * Wired by Sp00kyClient to push status changes into DevTools. Kept as a
   * settable field (rather than a constructor arg) because DevTools is
   * constructed after DataModule.
   */
  public onQueryStatusChange?: (hash: QueryHash, status: QueryStatus) => void;
  /**
   * Optional observer invoked when a still-subscribed query's TTL heartbeat
   * fires (~90% of the TTL). Wired by Sp00kyClient to
   * `Sp00kySync.heartbeatQuery`, which refreshes the remote `_00_query`
   * row's `lastActiveAt` so an actively-watched query never expires. Settable
   * field (not a constructor arg) because the sync engine is wired after
   * DataModule is constructed — mirrors `onQueryStatusChange`.
   */
  public onHeartbeat?: (hash: QueryHash) => void;
  /**
   * Optional hook fired by {@link deregisterQuery} when an opt-in query (e.g. a
   * viewport-windowed list cancelling an off-screen window) loses its last
   * subscriber. Wired by Sp00kyClient to enqueue a `cleanup` down-event, which
   * tears the remote `_00_query` view down (releasing its `_00_list_ref` edges)
   * instead of leaving it for the TTL sweep. The local view + state are freed in
   * {@link finalizeDeregister} only after that remote delete, so a fast
   * re-subscribe (scroll back) can abort/heal the teardown — see `cleanupQuery`.
   */
  public onDeregister?: (hash: QueryHash) => void;
  // Salt for query-id hashing. Set from SurrealDB's session::id() so two
  // browser sessions registering the same logical query (same surql + params)
  // don't collide on the same `_00_query` row — each session gets its own.
  // Empty string until init(sessionId) is called.
  private sessionId: string = '';
  // Authenticated user record id (e.g. `"user:abc"`). Updated by
  // `setCurrentUserId` from the auth subscription. null when
  // unauthenticated. Consulted by `Sp00kySync.listRefTable()` so the
  // poll and LIVE subscription target the same per-user
  // `_00_list_ref_user_<id>` table the sync engine writes to.
  private currentUserId: string | null = null;

  constructor(
    private cache: CacheModule,
    private local: LocalStore,
    private schema: S,
    logger: Logger,
    // Client-side SSP aggregation throttle: coalesces the in-browser
    // StreamProcessor's per-record stream updates per query before notifying
    // readers, so a burst of synced records repaints the UI once per window
    // rather than row-by-row. 50ms keeps it responsive while still batching the
    // initial-sync stream. Override via `SyncedDbConfig.streamDebounceTime`.
    private streamDebounceTime: number = 50
  ) {
    this.logger = logger.child({ service: 'DataModule' });
  }

  async init(sessionId: string): Promise<void> {
    this.sessionId = sessionId;
    this.logger.info(
      { sessionId, Category: 'sp00ky-client::DataModule::init' },
      'DataModule initialized'
    );
  }

  /**
   * Update the session salt used in query-id hashing. Call this when the
   * SurrealDB session changes (sign-in, sign-out, reconnect). Subsequently
   * registered queries will get fresh, session-scoped IDs.
   */
  setSessionId(sessionId: string): void {
    this.sessionId = sessionId;
  }

  /** Shared-tabs: bake this tab's identity into mutation ids so a rollback of
   *  a follower's mutation routes back to the tab that made it. */
  setTabId(tabId: string): void {
    this.tabId = tabId;
  }

  /**
   * Update the authenticated user record id. Pass `null` on sign-out.
   * Read by `Sp00kySync.listRefTable()` so the LIVE subscription and
   * the poll route to the same per-user `_00_list_ref_user_<id>` the
   * SSP writes to.
   */
  setCurrentUserId(userId: string | null): void {
    this.currentUserId = userId;
  }

  /** Read-only view of the authenticated user id used for per-user
   * `_00_list_ref` routing. Other modules consult this so they pick the
   * same table name DataModule does. */
  getCurrentUserId(): string | null {
    return this.currentUserId;
  }

  // ==================== QUERY MANAGEMENT ====================

  /**
   * Register a query and return its hash for subscriptions
   */
  async query<T extends TableNames<S>>(
    tableName: T,
    surqlString: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    plan?: QueryPlan
  ): Promise<QueryHash> {
    const hash = await this.calculateHash({ surql: surqlString, params });
    this.logger.debug(
      { hash, Category: 'sp00ky-client::DataModule::query' },
      'Query Initialization: started'
    );

    // `_00_query` stays the single shared registration table in both
    // ref-modes; the per-user split happens only on `_00_list_ref`.
    const recordId = new RecordId('_00_query', hash);

    if (this.activeQueries.has(hash)) {
      this.logger.debug(
        { hash, Category: 'sp00ky-client::DataModule::query' },
        'Query Initialization: exists, returning'
      );
      return hash;
    }

    // Another call is already creating this query — wait for it
    if (this.pendingQueries.has(hash)) {
      this.logger.debug(
        { hash, Category: 'sp00ky-client::DataModule::query' },
        'Query Initialization: pending, waiting for existing creation'
      );
      await this.pendingQueries.get(hash);
      return hash;
    }

    this.logger.debug(
      { hash, Category: 'sp00ky-client::DataModule::query' },
      'Query Initialization: not found, creating new query'
    );

    // Create the query and track the pending promise
    const promise = this.createAndRegisterQuery<T>(
      hash,
      recordId,
      surqlString,
      params,
      ttl,
      tableName,
      plan,
      await this.calculateMembershipKey({ surql: surqlString, params })
    );
    this.pendingQueries.set(hash, promise);
    try {
      await promise;
    } finally {
      this.pendingQueries.delete(hash);
    }

    return hash;
  }

  /**
   * Subscribe to query updates
   */
  subscribe(
    queryHash: string,
    callback: QueryUpdateCallback,
    options: { immediate?: boolean } = {}
  ): () => void {
    if (!this.subscriptions.has(queryHash)) {
      this.subscriptions.set(queryHash, new Set());
    }

    this.subscriptions.get(queryHash)?.add(callback);

    if (options.immediate) {
      const query = this.activeQueries.get(queryHash);
      if (query) {
        callback(query.records);
      }
    }

    // Return unsubscribe function
    return () => {
      const subs = this.subscriptions.get(queryHash);
      if (subs) {
        subs.delete(callback);
        if (subs.size === 0) {
          this.subscriptions.delete(queryHash);
          // NOTE: intentionally do NOT tear down the query / free its in-browser
          // SSP view here. The subscriber-gated heartbeat (startTTLHeartbeat)
          // already self-stops once subscribers hit 0, so an abandoned query
          // stops being kept alive and the server's TTL sweep removes it. Freeing
          // the local view on every last-unsubscribe caused re-registration churn
          // on navigation (open A → leave → open A again re-registers), which is
          // a flakiness risk for no real benefit — keep the local view resident.
        }
      }
    };
  }

  /**
   * Subscribe to a query's fetch-status changes (idle/fetching).
   * With `{ immediate: true }` the callback fires synchronously with the
   * current status (defaults to `idle` if the query isn't registered yet).
   */
  subscribeStatus(
    queryHash: string,
    callback: QueryStatusCallback,
    options: { immediate?: boolean } = {}
  ): () => void {
    if (!this.statusSubscriptions.has(queryHash)) {
      this.statusSubscriptions.set(queryHash, new Set());
    }
    this.statusSubscriptions.get(queryHash)?.add(callback);

    if (options.immediate) {
      callback(this.activeQueries.get(queryHash)?.status ?? 'idle');
    }

    return () => {
      const subs = this.statusSubscriptions.get(queryHash);
      if (subs) {
        subs.delete(callback);
        if (subs.size === 0) {
          this.statusSubscriptions.delete(queryHash);
        }
      }
    };
  }

  /**
   * Set a query's fetch status and notify status observers (DevTools +
   * `subscribeStatus` listeners). No-op when the status is unchanged or the
   * query is unknown.
   */
  setQueryStatus(queryHash: string, status: QueryStatus): void {
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState || queryState.status === status) {
      return;
    }
    queryState.status = status;

    this.onQueryStatusChange?.(queryHash, status);

    const subs = this.statusSubscriptions.get(queryHash);
    if (subs) {
      for (const callback of subs) {
        callback(status);
      }
    }
  }

  /**
   * Enter a fetch cycle for a query. Refcounted: registration and concurrent
   * poll/LIVE sync rounds can overlap on the same hash, and only the OUTERMOST
   * cycle may flip the status — 0→1 emits `fetching`, and `endFetching`'s 1→0
   * emits `idle`. Always pair with `endFetching` in a `finally`.
   */
  beginFetching(queryHash: string): void {
    const depth = this.fetchDepth.get(queryHash) ?? 0;
    this.fetchDepth.set(queryHash, depth + 1);
    if (depth === 0) {
      this.setQueryStatus(queryHash, 'fetching');
    }
  }

  /** Leave a fetch cycle started with {@link beginFetching}; emits `idle` on the last exit. */
  endFetching(queryHash: string): void {
    const depth = this.fetchDepth.get(queryHash) ?? 0;
    if (depth <= 1) {
      this.fetchDepth.delete(queryHash);
      this.setQueryStatus(queryHash, 'idle');
      return;
    }
    this.fetchDepth.set(queryHash, depth - 1);
  }

  /**
   * Subscribe to mutations (for sync)
   */
  onMutation(callback: MutationCallback): () => void {
    this.mutationCallbacks.add(callback);
    return () => {
      this.mutationCallbacks.delete(callback);
    };
  }

  /**
   * Handle stream updates from DBSP (via CacheModule)
   */
  async onStreamUpdate(update: StreamUpdate): Promise<void> {
    const { queryHash, op } = update;

    // DELETE propagates immediately — a removed row should disappear without
    // waiting on a debounce.
    //
    // CREATE and UPDATE are coalesced per query on a trailing timer. A list's
    // rows stream in from sync as many small `_00_list_ref` diffs, each its own
    // `cache.saveBatch` → one stream update per chunk. `ingestMany` only
    // coalesces records ingested synchronously together, so chunks spread over
    // time each re-materialize and re-notify — a 50-row window can fire 30+
    // updates as it fills. Each StreamUpdate carries the full materialized
    // `localArray`, so the latest one already reflects every prior chunk: keep
    // only it and fire once on the trailing edge, settling the query in a couple
    // of notifications instead of one per chunk.
    if (op === 'DELETE') {
      const existing = this.debounceTimers.get(queryHash);
      if (existing) {
        clearTimeout(existing);
        this.debounceTimers.delete(queryHash);
      }
      // The DELETE update carries the full latest localArray, so the coalesced
      // CREATE/UPDATE it supersedes is already reflected — drop it.
      this.pendingStreamUpdates.delete(queryHash);
      await this.processStreamUpdate(update);
      return;
    }

    this.queueStreamUpdate(update);
  }

  /** Coalesce `update` onto the query's trailing timer (see onStreamUpdate). */
  private queueStreamUpdate(update: StreamUpdate): void {
    const { queryHash } = update;
    // Clear existing timer if any
    if (this.debounceTimers.has(queryHash)) {
      // oxlint-disable-next-line no-non-null-assertion -- guarded by .has() check above
      clearTimeout(this.debounceTimers.get(queryHash)!);
    }

    // Set new timer
    this.pendingStreamUpdates.set(queryHash, update);
    const timer = setTimeout(async () => {
      this.debounceTimers.delete(queryHash);
      this.pendingStreamUpdates.delete(queryHash);
      await this.processStreamUpdate(update);
    }, this.streamDebounceTime);

    this.debounceTimers.set(queryHash, timer);
  }

  /**
   * Re-materialize + notify a query whose MEMBERSHIP changed without any row
   * needing to be fetched, i.e. without the SSP stream update that normally
   * carries the notify. That is every row this client wrote itself: the local
   * CREATE memoized it at `_00_rv = 1`, the server publishes it at 1, so the
   * sync engine rightly fetches nothing - and then nobody told the subscribers
   * that `remoteArray` now holds the id. The row appeared on reload only.
   *
   * Routed through the same per-query debounce as a real stream update, so it
   * cannot race one: a pending real update already materializes against the
   * current `remoteArray` and wins. The synthetic update re-uses the circuit's
   * last `localArray` and skips the persist/metrics that describe an ingest.
   */
  scheduleRematerialize(queryHash: string): void {
    if (this.debounceTimers.has(queryHash)) return;
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) return;
    this.queueStreamUpdate({
      queryHash,
      localArray: queryState.config.localArray ?? [],
      op: 'UPDATE',
      synthetic: true,
    });
  }

  /**
   * Process a query's pending (debounced) stream update NOW instead of on the
   * trailing edge. Called by the sync engine before it flips a query back to
   * `idle`, so the status change never races ahead of the rows it fetched.
   * No-op when nothing is pending. The pending entry is removed before the
   * await so a concurrently-firing timer can't process it twice.
   */
  async flushPendingStreamUpdate(queryHash: string): Promise<boolean> {
    const timer = this.debounceTimers.get(queryHash);
    if (timer) {
      clearTimeout(timer);
      this.debounceTimers.delete(queryHash);
    }
    const pending = this.pendingStreamUpdates.get(queryHash);
    if (!pending) return false;
    this.pendingStreamUpdates.delete(queryHash);
    await this.processStreamUpdate(pending);
    return true;
  }

  /**
   * Materialize a query's result rows from the local store.
   *
   * A query's rows are its MEMBERSHIP — the id-set the server put in
   * `_00_list_ref` (`remoteArray`) — not "every local body that matches the
   * WHERE". Those two disagree, and the disagreement was the bug: when a row
   * leaves a query's window but still exists upstream, `handleRemovedRecords`
   * keeps its local body and never re-fetches it, so a predicate re-scan finds
   * that stale body still matching and keeps rendering the row. Selecting the
   * id-set directly is also the only correct thing for a windowed query, where
   * re-applying `START m` against the shared local store skips the window's own
   * rows entirely (sparse windowing) and returns nothing.
   *
   * The rendered set is:
   *
   *     (membership ∪ (pendingWrites ∩ localArray)) − pendingDeletes
   *
   * The middle term keeps optimistic writes visible without re-admitting stale
   * rows. Every local write is fed to the SSP (`cache.saveBatch` →
   * `ingestMany`), so `localArray` answers "does this row match the predicate
   * per LOCAL truth". A pending write that moves a row into the window is in
   * `localArray` and shows; one that moves a row out is absent and does not; a
   * stale body the server dropped has no pending write at all, so it stays out.
   * `pendingDeletes` covers the reverse lag — the server still lists a row whose
   * DELETE is sitting in our outbox.
   *
   * Falls back to the predicate scan only when membership has never been
   * established (a query first run on this device), so an offline first paint
   * still shows something.
   */
  private async materializeRecords(
    queryState: QueryState,
    sspArray?: Array<[string, number]>
  ): Promise<Record<string, any>[]> {
    const t0 = performance.now();
    const records = await this.materializeFromConfig(queryState.config, sspArray);
    // Local SurrealDB record-fetch time → DevTools "localFetch" phase.
    this.recordPhase(queryState, 'localFetch', performance.now() - t0);
    return records;
  }

  /**
   * The materialization itself, without the DevTools timing wrapper. Split out so
   * cold-start seeding can use it before a `QueryState` exists.
   */
  private async materializeFromConfig(
    config: QueryConfig,
    sspArray?: Array<[string, number]>
  ): Promise<Record<string, any>[]> {
    const plan = config.plan;
    const membership = this.resolveMembership(config, sspArray);
    let records: Record<string, any>[];

    if (membership) {
      const ids = await this.buildRenderIds(config, membership, sspArray);
      if (plan) {
        records = await this.local.select(buildIdSetPlan(plan, ids), config.params);
      } else {
        const idSetSurql = buildIdSetSurql(config.surql);
        if (idSetSurql) {
          const [rows] = await this.local.query<[Record<string, any>[]]>(idSetSurql.query, {
            ...config.params,
            __win: ids,
          });
          records = rows || [];
        } else {
          // Unparseable shape (no top-level FROM): the predicate scan is the
          // only option left. Rare — the query-builder always supplies a plan.
          const [rows] = await this.local.query<[Record<string, any>[]]>(
            config.surql,
            config.params
          );
          records = rows || [];
        }
      }
    } else if (plan) {
      records = await this.local.select(plan, config.params);
    } else {
      const [rows] = await this.local.query<[Record<string, any>[]]>(config.surql, config.params);
      records = rows || [];
    }
    return records;
  }

  /**
   * The authoritative membership list to render from, or `null` when membership
   * has never been established and the caller must fall back to a scan.
   *
   * A windowed query has no usable fallback — re-running its `START m` locally
   * returns the wrong rows — so it renders from whatever id-set is on hand
   * (SSP's included) rather than degrading to a scan. That is the pre-existing
   * behavior for windows and is preserved.
   */
  private resolveMembership(
    config: QueryConfig,
    sspArray?: Array<[string, number]>
  ): RecordVersionArray | null {
    if (config.membershipKnown) return config.remoteArray ?? [];
    if (buildWindowMaterialization(config.surql) !== null) {
      return (
        (config.remoteArray?.length && config.remoteArray) ||
        (sspArray?.length && sspArray) ||
        config.localArray ||
        []
      );
    }
    return null;
  }

  // ---- Settled-write grace window ----------------------------------------
  //
  // A local write is visible because it is in the outbox: the render set is
  // `(membership ∪ (pendingWrites ∩ localArray)) − pendingDeletes`. The outbox
  // row is deleted the moment the remote push succeeds (see
  // `UpQueue.next` / `SyncScheduler` — the delete is deliberately tied to the
  // push, not to anything downstream), but the row does not enter `membership`
  // until the SSP has ingested it, materialized the view and written the
  // `_00_list_ref` edge, and this client has read that back.
  //
  // Between those two moments the row is in neither term, so it is rendered,
  // then disappears, then returns — reported as "the comment shows on every
  // other client but not on the one that wrote it". Other clients don't blink
  // because a client that never established membership renders from the local
  // predicate scan instead.
  //
  // So a settled write keeps its place in the union for a short grace period,
  // until membership catches up or the deadline passes. Kept in memory only:
  // it covers a round trip, and a reload re-derives membership anyway.
  private readonly settledWrites = new Map<string, number>();
  private readonly settledDeletes = new Map<string, number>();

  // ---- Outbox id cache -----------------------------------------------------
  //
  // `getPendingRecordIds` runs a full `SELECT ... FROM _00_pending_mutations`,
  // and `buildRenderIds` calls it on EVERY materialization of EVERY query. That
  // is a round trip down the local engine's single-flight op queue, so with tens
  // of live queries one ingest fanned out into tens of them — in a 4-second
  // capture, 11 of 25 local queries were that one statement, against an outbox
  // holding zero rows.
  //
  // Cached, and invalidated at the two points that change the outbox: enqueueing
  // a mutation (below) and `noteWriteSettled` (the row is deleted before it is
  // called). The TTL is a BACKSTOP, not the mechanism: should some other path
  // ever remove a row without telling us, this bounds the staleness to one tick
  // instead of the session. Well under the grace windows the render set already
  // tolerates deliberately (SETTLED_WRITE_GRACE_MS is 10s).
  private pendingIds: { writes: Set<string>; deletes: Set<string> } | null = null;
  private pendingIdsAt = 0;
  private pendingIdsInflight: Promise<{ writes: Set<string>; deletes: Set<string> }> | null = null;
  private static readonly PENDING_IDS_TTL_MS = 250;
  // Bumped by every invalidation. A read carries the generation it started
  // under, and a result from an older generation is neither cached nor
  // returned to a caller that asked after the invalidation: it was taken
  // before the outbox row it needs existed. Without this, a materialization
  // that joined a read already in flight when `create()` invalidated saw the
  // pre-create set, found "no change", and skipped its notify - the one
  // stream update a CREATE gets, so the row stayed invisible until reload.
  private pendingIdsGen = 0;
  private static readonly PENDING_IDS_MAX_REREADS = 3;

  /** Drop the cached outbox ids. Cheap; call it on anything that could change
   *  `_00_pending_mutations`. */
  private invalidatePendingIds(): void {
    this.pendingIds = null;
    this.pendingIdsGen++;
    // The in-flight read (if any) predates this invalidation; whoever joins
    // next starts a fresh one. Its own joiners re-read via the generation check.
    this.pendingIdsInflight = null;
  }

  /**
   * Grace period for a settled write. Long enough to cover an SSP round trip
   * that is running slowly (seconds, not milliseconds, when the edge path is
   * backed up), short enough that a write the server silently dropped cannot
   * linger misleadingly.
   *
   * The rejection case does NOT rely on this expiring: an application error
   * rolls the mutation back and never reports it settled, so it vanishes at
   * once. This deadline only bounds the case where the write succeeded and its
   * membership never arrived at all.
   *
   * 30 s, up from 10 s. At 10 s every server hiccup longer than a scheduler
   * drain plus one slow database commit showed up as the user's own chat
   * message vanishing and coming back a minute later. 30 s covers a drain and
   * a stalled commit end to end; a membership that takes longer than that is
   * a server fault the admin heartbeat reports, not something to paper over.
   */
  private static readonly SETTLED_WRITE_GRACE_MS = 30_000;

  /**
   * Whether any settled write or delete is still waiting for membership to
   * catch up. The sync poll keeps its fast cadence while this is true, so a
   * missed LIVE notification for the edge is picked up on the next poll tick
   * rather than after the idle backoff.
   */
  hasSettledWritesPending(): boolean {
    this.pruneSettled(Date.now());
    return this.settledWrites.size > 0 || this.settledDeletes.size > 0;
  }

  /**
   * Report that a mutation was accepted by the server and its outbox row
   * removed. Called only on the SUCCESS path — a rolled-back mutation must
   * disappear immediately, which is what makes this safe.
   */
  noteWriteSettled(recordId: string, mutationType: string): void {
    // The outbox row is already gone by the time this is called — that is what
    // makes it the delete-side invalidation point for the cache above.
    this.invalidatePendingIds();
    const until = Date.now() + DataModule.SETTLED_WRITE_GRACE_MS;
    if (mutationType === 'delete') this.settledDeletes.set(recordId, until);
    else this.settledWrites.set(recordId, until);
  }

  /** Drop entries past their deadline. */
  private pruneSettled(now: number): void {
    for (const [id, until] of this.settledWrites) {
      if (until <= now) this.settledWrites.delete(id);
    }
    for (const [id, until] of this.settledDeletes) {
      if (until <= now) this.settledDeletes.delete(id);
    }
  }

  /** Apply the pending-write union and pending-delete subtraction, and map to
   *  RecordIds for the engines' id-set path. */
  private async buildRenderIds(
    config: QueryConfig,
    membership: RecordVersionArray,
    sspArray?: Array<[string, number]>
  ): Promise<unknown[]> {
    const { writes, deletes } = await this.getPendingRecordIds();
    const now = Date.now();
    this.pruneSettled(now);
    // A settled write counts as pending until membership catches up. Merged
    // into the same sets so the union/subtraction below is unchanged — the
    // grace window changes WHEN an id leaves the render set, never how the
    // set is composed.
    if (this.settledWrites.size > 0) {
      for (const id of this.settledWrites.keys()) writes.add(id);
    }
    if (this.settledDeletes.size > 0) {
      for (const id of this.settledDeletes.keys()) deletes.add(id);
    }

    const ordered: string[] = [];
    const seen = new Set<string>();
    for (const [id] of membership) {
      // Membership has caught up with this write: the grace window has done
      // its job and ends here rather than at its deadline. Doing it inline
      // keeps the common case (nothing settled) free of extra passes.
      if (this.settledWrites.size > 0) this.settledWrites.delete(id);
      if (deletes.has(id) || seen.has(id)) continue;
      seen.add(id);
      ordered.push(id);
    }
    // A settled DELETE is the mirror case: membership still lists the row
    // until the SSP publishes its removal, so the id stays subtracted until
    // membership stops naming it.
    if (this.settledDeletes.size > 0) {
      const stillListed = new Set(membership.map(([id]) => id));
      for (const id of [...this.settledDeletes.keys()]) {
        if (!stillListed.has(id)) this.settledDeletes.delete(id);
      }
    }
    if (writes.size > 0) {
      // Only pending writes the SSP agrees currently match this query — see the
      // formula in `materializeRecords`. `sspArray` is the fresher signal when a
      // stream update triggered this pass; `localArray` is the persisted one.
      const localView = (sspArray?.length && sspArray) || config.localArray || [];
      for (const [id] of localView) {
        if (!writes.has(id) || deletes.has(id) || seen.has(id)) continue;
        seen.add(id);
        ordered.push(id);
      }
    }
    // `_00_list_ref` is selected without an ORDER BY, so this id-set arrives in
    // whatever order the server happened to return. For a query with its own
    // ORDER BY that does not matter (the engine sorts), and for a window the
    // id-set order IS the window's slice order and must be preserved. Anything
    // else renders in server order while its first paint came from the local
    // scan in id order — the same rows, visibly reshuffled a second later.
    // Sorting here is what makes the two paints agree.
    const hasExplicitOrder = (config.plan?.orderBy?.length ?? 0) > 0;
    const isWindow = buildWindowMaterialization(config.surql) !== null;
    if (!hasExplicitOrder && !isWindow) {
      ordered.sort();
    }
    return ordered.map((id) => parseRecordIdString(id));
  }

  private async processStreamUpdate(update: StreamUpdate): Promise<void> {
    const { queryHash, localArray, materializationTimeMs } = update;
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
        'Received update for unknown query. Skipping...'
      );
      return;
    }

    // Update the rolling materialization-sample window before the work that
    // could throw, so the percentiles still move when the downstream local
    // query fails (the materialization step itself ran).
    if (typeof materializationTimeMs === 'number') {
      queryState.materializationSamples.push(materializationTimeMs);
      if (queryState.materializationSamples.length > MATERIALIZATION_SAMPLE_WINDOW) {
        queryState.materializationSamples.shift();
      }
      queryState.lastIngestLatencyMs = materializationTimeMs;
    }
    // Record the SSP internal sub-phase timings (from the WASM binding) so
    // DevTools can attribute ingest cost to store-apply vs circuit-step vs transform.
    if (typeof update.storeApplyMs === 'number')
      this.recordPhase(queryState, 'sspStoreApply', update.storeApplyMs);
    if (typeof update.circuitStepMs === 'number')
      this.recordPhase(queryState, 'sspCircuitStep', update.circuitStepMs);
    if (typeof update.transformMs === 'number')
      this.recordPhase(queryState, 'sspTransform', update.transformMs);
    const percentiles = this.computeMaterializationPercentiles(queryState.materializationSamples);

    // Fence against bucket switches: this update's `localArray` came from the
    // pre-switch SSP circuit; applying it after a switch would show (and
    // persist) the previous user's ids in the new bucket.
    const epoch = this.local.epoch;

    try {
      // Materialize the query's rows. For a windowed (offset) query, re-running
      // the original surql would re-apply `START n` against the shared local DB
      // and skip the window's rows entirely; instead select the SSP's
      // materialized window id-set (`localArray`) directly, re-applying the
      // original ORDER BY for stable display order. Non-offset queries keep the
      // normal re-query path.
      const newRecords = await this.materializeRecords(queryState, localArray);
      if (epoch !== this.local.epoch) return;
      if (!update.synthetic) queryState.config.localArray = localArray;

      const prevJson = JSON.stringify(queryState.records);
      const newJson = JSON.stringify(newRecords);
      queryState.records = newRecords;
      const recordsChanged = prevJson !== newJson;

      // updateCount counts user-visible updates (matches the prior semantic),
      // while the materialization sample/percentiles already moved above for
      // every observed engine step.
      if (recordsChanged) {
        queryState.updateCount++;
        queryState.lastUpdatedAt = Date.now();
      }

      // A synthetic re-materialize did not come from an ingest: nothing about
      // the circuit's view changed, so there is nothing to persist for it.
      if (!update.synthetic) {
        await this.local.query(
          surql.seal(
            surql.updateSet('id', [
              'localArray',
              'rowCount',
              'updateCount',
              'lastIngestLatency',
              'materializationP55',
              'materializationP90',
              'materializationP99',
            ])
          ),
          {
            id: queryState.config.id,
            localArray,
            rowCount: localArray.length,
            updateCount: queryState.updateCount,
            lastIngestLatency: queryState.lastIngestLatencyMs,
            materializationP55: percentiles.p55,
            materializationP90: percentiles.p90,
            materializationP99: percentiles.p99,
          },
          { epoch }
        );
      }

      if (!recordsChanged) {
        this.logger.debug(
          { queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
          'Query records unchanged, skipping notification'
        );
        return;
      }

      // Notify subscribers
      const subscribers = this.subscriptions.get(queryHash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }

      this.logger.debug(
        {
          queryHash,
          recordCount: newRecords?.length,
          Category: 'sp00ky-client::DataModule::onStreamUpdate',
        },
        'Query updated from stream'
      );
    } catch (err) {
      if (err instanceof StaleEpochError) {
        this.logger.debug(
          { queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
          'Dropped stream update from before a bucket switch'
        );
        return;
      }
      queryState.errorCount++;
      this.logger.error(
        { err, queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
        'Failed to fetch records for stream update'
      );
      // Best-effort persist of the bumped errorCount; swallow secondary
      // failures to avoid masking the original error in logs.
      try {
        await this.local.query(surql.seal(surql.updateSet('id', ['errorCount'])), {
          id: queryState.config.id,
          errorCount: queryState.errorCount,
        });
      } catch (persistErr) {
        this.logger.warn(
          {
            err: persistErr,
            queryHash,
            Category: 'sp00ky-client::DataModule::onStreamUpdate',
          },
          'Failed to persist incremented errorCount'
        );
      }
    }
  }

  /**
   * Compute p55/p90/p99 from a rolling window of materialization samples.
   * Returns nulls for any percentile that has no samples yet so SurrealDB
   * `option<float>` columns stay NONE rather than 0 before the first ingest.
   */
  private computeMaterializationPercentiles(samples: number[]): {
    p55: number | null;
    p90: number | null;
    p99: number | null;
  } {
    if (samples.length === 0) {
      return { p55: null, p90: null, p99: null };
    }
    const sorted = [...samples].sort((a, b) => a - b);
    const pick = (q: number) => {
      const idx = Math.min(sorted.length - 1, Math.floor(q * sorted.length));
      return sorted[idx]!;
    };
    return { p55: pick(0.55), p90: pick(0.9), p99: pick(0.99) };
  }

  /** Record a per-phase timing sample (ms) on a query's rolling window. */
  private recordPhase(qs: QueryState, phase: string, ms: number): void {
    if (!Number.isFinite(ms)) return;
    const arr = qs.phaseSamples[phase] ?? (qs.phaseSamples[phase] = []);
    pushSample(arr, ms);
    qs.phaseLast[phase] = ms;
  }

  /** Record the remote record-fetch time (ms) for a query. Called by the sync engine. */
  recordRemoteFetch(hash: string, ms: number): void {
    const qs = this.activeQueries.get(hash);
    if (qs) this.recordPhase(qs, 'remoteFetch', ms);
  }

  /**
   * Record the frontend reconcile time (ms) for a query. Called from `useQuery`
   * via `Sp00kyClient.reportFrontendTiming` after it applies an update to its store.
   */
  recordFrontendTiming(hash: string, ms: number): void {
    const qs = this.activeQueries.get(hash);
    if (qs) this.recordPhase(qs, 'frontend', ms);
  }

  /**
   * Build the per-query processing-time breakdown surfaced to the DevTools panel
   * and the MCP. `ssp` is the WASM-ingest wall time (from `materializationSamples`);
   * the rest come from the per-phase rolling windows + one-shot registration timings.
   */
  phaseTimings(q: QueryState): QueryTimings {
    const stat = (phase: string) =>
      phaseStatOf(q.phaseSamples[phase] ?? [], q.phaseLast[phase] ?? null);
    return {
      ssp: phaseStatOf(q.materializationSamples, q.lastIngestLatencyMs),
      sspStoreApply: stat('sspStoreApply'),
      sspCircuitStep: stat('sspCircuitStep'),
      sspTransform: stat('sspTransform'),
      localFetch: stat('localFetch'),
      remoteFetch: stat('remoteFetch'),
      frontend: stat('frontend'),
      registration: q.registrationTimings,
      updateCount: q.updateCount,
      errorCount: q.errorCount,
    };
  }

  /**
   * Get query state (for sync and devtools)
   */
  getQueryByHash(hash: string): QueryState | undefined {
    return this.activeQueries.get(hash);
  }

  /**
   * Cold-query guard for instant-hydrate: true when the query exists, hasn't been
   * hydrated, and has NOT yet fetched its server result (`remoteArray` empty).
   * We gate on `remoteArray`, not local `records`: a windowed query is often
   * partially pre-seeded from the circuit (e.g. the dashboard's 5-row preview),
   * but it still hasn't loaded its own full window from the server — so it should
   * still hydrate. A warm re-subscribe (remoteArray already populated) is skipped.
   */
  isCold(hash: string): boolean {
    const qs = this.activeQueries.get(hash);
    return !!qs && !qs.hydrated && (qs.config.remoteArray?.length ?? 0) === 0;
  }

  /**
   * Walk a hydrated record's fields and append any EMBEDDED child records to
   * `batch` (recursing for nested related fields). An embedded child is a
   * value that is itself a record — a non-null object whose `id` is a
   * `RecordId` — or an array of such records (one-to-many vs one-to-one). A
   * bare `RecordId` (a foreign-key reference) or any other value is skipped,
   * so this never mistakes a FK column for an embedded body. Children are
   * keyed by their own `record.id.table`, versioned by `_00_rv`, and cleaned
   * to their table's real columns (which strips the alias/related fields).
   * `seen` dedupes within the batch.
   */
  private collectEmbeddedChildren(
    record: Record<string, any>,
    batch: CacheRecord[],
    seen: Set<string>
  ): void {
    const isEmbeddedRecord = (v: unknown): v is RecordWithId =>
      !!v &&
      typeof v === 'object' &&
      !(v instanceof RecordId) &&
      (v as { id?: unknown }).id instanceof RecordId;

    for (const value of Object.values(record)) {
      const children = Array.isArray(value)
        ? value.filter(isEmbeddedRecord)
        : isEmbeddedRecord(value)
          ? [value]
          : [];
      for (const child of children) {
        const key = encodeRecordId(child.id);
        if (seen.has(key)) continue;
        seen.add(key);
        // Recurse FIRST so nested grandchildren are captured before `cleanRecord`
        // strips this child's alias fields.
        this.collectEmbeddedChildren(child, batch, seen);
        const table = child.id.table.toString();
        const tableSchema = this.schema.tables.find((t) => t.name === table);
        batch.push({
          table,
          op: 'CREATE',
          // Flatten the child too: a nested comment carries its own embedded
          // `author` object which the schemafull `record<user>` field would
          // reject. (Its author is captured as a separate row by the recursion
          // above.)
          record: this.flattenRelationsForStorage(child, tableSchema),
          version: (child._00_rv as number) || 1,
        });
      }
    }
  }

  /**
   * Prepare a subquery-bearing row (preload / hydration) for the schemafull
   * local store: replace an embedded FORWARD-relation object (`author = { id, … }`)
   * with its RecordId so a `record<…>` field coerces, and DROP reverse-subquery
   * ARRAYS (`comments = [ … ]`) since their rows are cached separately as their
   * own bodies. A flat record — as the live `SELECT * FROM $ids` sync returns,
   * with relations already RecordIds — passes through unchanged.
   */
  private flattenRelationsForStorage(
    record: Record<string, any>,
    tableSchema?: { columns: Record<string, any> }
  ): RecordWithId {
    const isEmbeddedRecord = (v: unknown): boolean =>
      !!v &&
      typeof v === 'object' &&
      !(v instanceof RecordId) &&
      (v as { id?: unknown }).id instanceof RecordId;
    const flat: Record<string, any> = {};
    for (const [k, v] of Object.entries(record)) {
      if (isEmbeddedRecord(v)) {
        flat[k] = (v as { id: unknown }).id;
      } else if (Array.isArray(v) && v.some(isEmbeddedRecord)) {
        continue;
      } else {
        flat[k] = v;
      }
    }
    return (tableSchema ? cleanRecord(tableSchema.columns, flat) : flat) as RecordWithId;
  }

  /**
   * Instant-hydrate: ingest rows fetched one-shot from the remote (the query's own
   * surql run directly) so the query DISPLAYS immediately, while the full realtime
   * registration proceeds in the background. Ingests with versions (`_00_rv`) so the
   * later `syncRecords` dedup skips re-pulling unchanged bodies, and seeds
   * `remoteArray` so windowed queries materialize the correct window (no sparse
   * local-circuit issue). Runs at most once per query (the `hydrated` flag).
   */
  async applyHydration(hash: string, rows: RecordWithId[]): Promise<void> {
    const queryState = this.activeQueries.get(hash);
    if (!queryState) return;
    queryState.hydrated = true; // run-once, even when the remote returns nothing
    if (rows.length === 0) return;

    const epoch = this.local.epoch;
    const tableName = queryState.config.tableName;
    await this.buildAndSaveCacheBatch(tableName, rows);
    // Bucket switched while we persisted: these rows were fetched under the
    // previous auth context — don't let them prime the new bucket's query
    // state; the rebind's re-registration refills it. (saveBatch's own epoch
    // fence usually catches this, but the switch can land between it and here.)
    if (epoch !== this.local.epoch) return;

    // Prime remoteArray from the hydrated id+version pairs: `materializeRecords`
    // renders from it and it feeds the version dedup. Registration later
    // overwrites it with the authoritative `_00_list_ref`.
    queryState.config.remoteArray = rows.map(
      (r) => [encodeRecordId(r.id), (r._00_rv as number) || 1] as [string, number]
    );
    // These rows ARE the server's answer to this query, so membership is now
    // known and rendering can stop falling back to a predicate scan. The durable
    // `_00_window` mirror is deliberately left to `updateQueryRemoteArray` (the
    // single write point) — the registration that follows lands there anyway.
    queryState.config.membershipKnown = true;

    queryState.records = await this.materializeRecords(queryState);
    const subscribers = this.subscriptions.get(hash);
    if (subscribers) {
      for (const cb of subscribers) cb(queryState.records);
    }
  }

  /**
   * Build the cache batch for a set of one-shot rows and persist it to the
   * local DB + in-browser SSP. Maps each row to a `CREATE` op on its own table
   * and extracts EMBEDDED related children (any nesting depth) as their own
   * records — a `.related()` query returns its children embedded, and a later
   * correlated re-materialization needs them present as standalone rows.
   * Shared by `applyHydration` (live registration) and `persistSnapshot`
   * (preload).
   */
  private async buildAndSaveCacheBatch(tableName: string, rows: RecordWithId[]): Promise<void> {
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    const batch: CacheRecord[] = rows.map((record) => ({
      table: tableName,
      op: 'CREATE' as const,
      // Flatten embedded relations first: a preload/hydration row can carry a
      // forward relation as a full nested OBJECT (`author = { id, … }`) and a
      // reverse subquery as an array of objects (`comments = [ … ]`). The
      // schemafull local field is `record<user>`, which rejects an object
      // (`Couldn't coerce … found { id: …, username: … }`) and would throw the
      // WHOLE batch. Store the parent with `author` as its RecordId and drop the
      // subquery arrays — the children are cached as their own rows below.
      record: this.flattenRelationsForStorage(record, tableSchema),
      version: (record._00_rv as number) || 1,
    }));

    const seen = new Set<string>(rows.map((r) => encodeRecordId(r.id)));
    for (const record of rows) {
      this.collectEmbeddedChildren(record, batch, seen);
    }

    await this.cache.saveBatch(batch);
  }

  /**
   * Preload/prewarm: persist one-shot rows (and their embedded related children)
   * into the local cache WITHOUT registering a query — no `activeQueries` entry,
   * no `_00_query` view, no TTL heartbeat. The rows live in the local DB as
   * ordinary bodies (never GC'd on their own) so a later `useQuery` seeds its
   * first paint from them instantly, then registers a live view to freshen.
   */
  async persistSnapshot(tableName: string, rows: RecordWithId[]): Promise<void> {
    if (rows.length === 0) return;
    await this.buildAndSaveCacheBatch(tableName, rows);
  }

  /**
   * Read the durable preload freshness marker for a query hash, or null if this
   * query was never preloaded in the current bucket. Co-located with the cached
   * rows (per-bucket `_00_preload` table) so a bucket switch that clears the
   * data also clears the marker — a stale marker can't claim "warm" when the
   * rows are gone. Any read error is treated as cold.
   */
  async getPreloadMarker(hash: string): Promise<{ fetchedAt: number; rowCount: number } | null> {
    try {
      // Pass a real RecordId: a bare string id hits the SurrealDB engine's
      // `FROM ONLY $__id` as a plain string, which "selects" the string itself
      // (a truthy non-row) instead of the record — misread as a warm marker.
      const row = await this.local.getById('_00_preload', new RecordId('_00_preload', hash));
      if (!row || typeof row !== 'object') return null;
      return {
        fetchedAt: Number((row as any).fetchedAt) || 0,
        rowCount: Number((row as any).rowCount) || 0,
      };
    } catch {
      return null;
    }
  }

  /** Stamp the preload freshness marker after a successful snapshot fetch. */
  async writePreloadMarker(hash: string, rowCount: number): Promise<void> {
    // RecordId, not a bare string: the SurrealDB engine binds the id verbatim,
    // and `UPSERT <string>` is an InternalError — the marker silently never
    // landed (the write is awaited inside preload's best-effort catch).
    await this.local.upsert(
      '_00_preload',
      new RecordId('_00_preload', hash),
      { fetchedAt: Date.now(), rowCount },
      'replace'
    );
  }

  // ---- Durable membership (`_00_window`) ---------------------------------
  //
  // A query's authoritative membership is the id-set the server put in
  // `_00_list_ref` — `config.remoteArray`. It is persisted on the `_00_query`
  // row, but that row's id is salted with `session::id()`, which is new on every
  // page load (and `''` offline), and `doSwitchBucket` wipes the table outright.
  // So membership never survived a reload: the first paint fell back to a
  // predicate scan over whatever bodies were still cached, which re-included
  // rows the server had already dropped from the window.
  //
  // `_00_window` fixes that: same data, keyed by a session-independent hash, in
  // a table nothing wipes. Mirrors the durable `_00_preload` marker above.
  //
  // Row shape: `{ ids, confirmed, updatedAt }`. `confirmed` is the one bit that
  // lets an EMPTY row be trusted on the next boot (see `getWindowMembership`).

  /**
   * Read the durable membership row, or `null` if this query has never had
   * authoritative membership on this device. Any read error is treated as
   * "unknown" so a broken row degrades to the predicate scan rather than
   * rendering an empty list.
   *
   * `confirmed` is true only for rows written after the server itself vouched
   * for the set (a non-empty id-set, or an empty one it reported a row count of
   * zero for, or an empty one that followed a non-empty one in the same
   * session). Rows written before the marker existed, including the `[]` rows a
   * pre-`ea56f50e` client mirrored from an unflushed read, read as unconfirmed.
   */
  async getWindowMembership(key: string): Promise<DurableMembership | null> {
    try {
      const row = await this.local.getById('_00_window', new RecordId('_00_window', key));
      if (!row || typeof row !== 'object') return null;
      const ids = (row as any).ids;
      if (!Array.isArray(ids)) return null;
      return { ids: ids as RecordVersionArray, confirmed: (row as any).confirmed === true };
    } catch {
      return null;
    }
  }

  /**
   * Persist the durable membership row. Best-effort: callers must not fail a
   * sync round because the mirror write failed.
   *
   * `confirmed` says whether a cold start may trust this row even when it is
   * empty. A confirmed empty is a real answer ("the server says this query has
   * no rows") and stays empty across a reload; an unconfirmed empty is the
   * retry budget's guess and falls back to the predicate scan on the next boot,
   * exactly as every empty row did before the marker existed.
   */
  async writeWindowMembership(
    key: string,
    ids: RecordVersionArray,
    confirmed: boolean
  ): Promise<void> {
    try {
      await this.local.upsert(
        '_00_window',
        new RecordId('_00_window', key),
        { ids, confirmed, updatedAt: Date.now() },
        'replace'
      );
    } catch (err) {
      this.logger.debug(
        { err, key, Category: 'sp00ky-client::DataModule::writeWindowMembership' },
        'Failed to persist window membership; it will be re-derived on next register'
      );
    }
  }

  /**
   * Record ids with a mutation still in the outbox, split by direction.
   *
   * Both halves feed {@link materializeRecords}: `writes` keeps optimistic
   * creates/updates visible before the server has acknowledged them, and
   * `deletes` suppresses rows the server still lists because our DELETE hasn't
   * been processed yet. Reading `_00_pending_mutations` (rather than tracking
   * ids in memory) is what makes both survive a reload.
   *
   * On failure returns empty sets: membership alone then decides, which can
   * briefly hide an optimistic write but never resurrects a deleted row.
   */
  async getPendingRecordIds(): Promise<{ writes: Set<string>; deletes: Set<string> }> {
    const now = Date.now();
    const cached = this.pendingIds;
    if (cached && now - this.pendingIdsAt < DataModule.PENDING_IDS_TTL_MS) {
      // COPIES: `buildRenderIds` merges the settled-write ids into these sets,
      // and handing out the cached instances would let it grow them permanently.
      return { writes: new Set(cached.writes), deletes: new Set(cached.deletes) };
    }
    // Single-flight: one ingest fans out to many queries materializing at once,
    // and without this they all queue their own identical read behind each other.
    // Bounded re-read: a result from a generation older than the one current
    // by the time it lands was taken before some outbox row existed, so it is
    // re-issued rather than returned. Bounded, because a write on every tick
    // must not spin this forever - the last read is returned then.
    let fresh: { writes: Set<string>; deletes: Set<string> } | null = null;
    for (let attempt = 0; attempt < DataModule.PENDING_IDS_MAX_REREADS; attempt++) {
      const gen = this.pendingIdsGen;
      if (!this.pendingIdsInflight) {
        const read = this.readPendingRecordIds(gen).finally(() => {
          // Only clear the slot this read still owns; an invalidation in the
          // meantime has replaced it (with null) for a newer read to fill.
          if (this.pendingIdsInflight === read) this.pendingIdsInflight = null;
        });
        this.pendingIdsInflight = read;
      }
      fresh = await this.pendingIdsInflight;
      if (gen === this.pendingIdsGen) break;
    }
    // oxlint-disable-next-line no-non-null-assertion -- the loop runs at least once
    return { writes: new Set(fresh!.writes), deletes: new Set(fresh!.deletes) };
  }

  /** The uncached read. Also the reload path after an invalidation, so the ids
   *  still survive a reload exactly as before. `gen` is the generation the read
   *  was issued under; the result is cached only if it is still current. */
  private async readPendingRecordIds(gen: number): Promise<{
    writes: Set<string>;
    deletes: Set<string>;
  }> {
    const writes = new Set<string>();
    const deletes = new Set<string>();
    try {
      const [rows] = await this.local.query<
        [{ recordId: RecordId<string>; mutationType: string }[]]
      >('SELECT recordId, mutationType FROM _00_pending_mutations');
      for (const row of rows ?? []) {
        if (!row?.recordId) continue;
        const id = encodeRecordId(row.recordId);
        if (row.mutationType === 'delete') deletes.add(id);
        else writes.add(id);
      }
    } catch (err) {
      this.logger.warn(
        { err, Category: 'sp00ky-client::DataModule::getPendingRecordIds' },
        'Failed to read pending mutations; optimistic writes may be briefly hidden'
      );
      // Do NOT cache a failed read: the empty sets are a fallback for this call,
      // not a statement that the outbox is empty.
      return { writes, deletes };
    }
    if (gen === this.pendingIdsGen) {
      this.pendingIds = { writes, deletes };
      this.pendingIdsAt = Date.now();
    }
    return { writes, deletes };
  }

  /** True while ≥1 live subscriber is watching this query (refcount guard). */
  hasSubscribers(hash: string): boolean {
    return (this.subscriptions.get(hash)?.size ?? 0) > 0;
  }

  /**
   * Opt-in eager teardown for a query whose LAST subscriber just left — used by
   * viewport-windowed lists to cancel off-screen windows instead of leaving
   * their remote views to expire on the TTL sweep. No-op while any subscriber
   * remains (refcount). Only enqueues the remote cleanup here; the local WASM
   * view + in-memory state are freed in {@link finalizeDeregister} after the
   * remote delete completes, so a re-subscribe in between aborts/heals it.
   *
   * NOTE: most queries should NOT use this — the default keep-alive on
   * unsubscribe avoids re-registration churn on navigation.
   */
  deregisterQuery(hash: string): void {
    if (this.hasSubscribers(hash)) return;
    if (!this.activeQueries.has(hash)) return;
    this.onDeregister?.(hash);
  }

  /**
   * Final local teardown after the remote `_00_query` row was deleted: free the
   * WASM view, heartbeat timer, debounce timer, and in-memory state. Caller
   * (`cleanupQuery`) guarantees no subscriber remains.
   */
  finalizeDeregister(hash: string): void {
    const qs = this.activeQueries.get(hash);
    if (qs?.ttlTimer) {
      clearTimeout(qs.ttlTimer);
      qs.ttlTimer = null;
    }
    const debounce = this.debounceTimers.get(hash);
    if (debounce) {
      clearTimeout(debounce);
      this.debounceTimers.delete(hash);
    }
    this.pendingStreamUpdates.delete(hash);
    this.fetchDepth.delete(hash);
    this.cache.unregisterQuery(hash);
    this.activeQueries.delete(hash);
    this.subscriptions.delete(hash);
  }

  /**
   * Get query state by id (for sync and devtools)
   */
  getQueryById(id: RecordId<string>): QueryState | undefined {
    return this.activeQueries.get(extractIdPart(id));
  }

  /**
   * Get all active queries (for devtools)
   */
  getActiveQueries(): QueryState[] {
    return Array.from(this.activeQueries.values());
  }

  getActiveQueryHashes(): QueryHash[] {
    return Array.from(this.activeQueries.keys());
  }

  async updateQueryLocalArray(id: string, localArray: RecordVersionArray): Promise<void> {
    const queryState = this.activeQueries.get(id);
    if (!queryState) {
      this.logger.warn(
        { id, Category: 'sp00ky-client::DataModule::updateQueryLocalArray' },
        'Query to update local array not found'
      );
      return;
    }
    const epoch = this.local.epoch;
    queryState.config.localArray = localArray;
    try {
      await this.local.query(
        surql.seal(surql.updateSet('id', ['localArray'])),
        {
          id: queryState.config.id,
          localArray,
        },
        { epoch }
      );
    } catch (err) {
      if (err instanceof StaleEpochError) return;
      throw err;
    }
  }

  async updateQueryRemoteArray(
    hash: string,
    remoteArray: RecordVersionArray,
    opts?: {
      /** `_00_query.rowCount` read in the same round trip; `null` = unknown. */
      serverRowCount?: number | null;
    }
  ): Promise<void> {
    const queryState = this.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn(
        { hash, Category: 'sp00ky-client::DataModule::updateQueryRemoteArray' },
        'Query to update remote array not found'
      );
      return;
    }
    // An empty id-set is only believable once a real one has arrived in this
    // session. `_00_list_ref` is published asynchronously — the SSP queues a
    // view's initial edges to a coalescing flusher and returns from
    // `fn::query::register` before they land — so an empty read right after
    // registration is routinely just "not flushed yet", for a query that has
    // rows. Taking it at face value latched `membershipKnown` on an empty set,
    // which renders nothing (no scan fallback), and mirrored `[]` into the
    // durable `_00_window` row, which kept the list blank across reloads.
    //
    // Ignoring it entirely (rather than storing `[]` with the latch withheld)
    // is deliberate: on a cold start `remoteArray` is seeded from the durable
    // row, and overwriting that with `[]` would blank the very rows the seed
    // exists to paint. The `_00_list_ref` poll re-reads within ~500ms and
    // delivers the real set.
    // `serverRowCount` is what makes the two cases separable. The SSP writes it
    // onto the `_00_query` row in the same statement that registers the view,
    // before it queues the view's initial edges — so `> 0` with no edges is the
    // flush window and `=== 0` is a genuinely empty query. `null`/undefined
    // means we could not read it (older server, row not visible yet).
    //
    // Retry counting cannot substitute for this: the poll runs 500ms after
    // registration, well inside the flush window for a real collection, so
    // "believe it the second time" blanked exactly the lists this guard exists
    // to protect — reported as rows vanishing ~2s after a page load.
    // Whether this write may be trusted by the NEXT session. Non-empty sets
    // always; an empty set only when the server stood behind it (a zero row
    // count, or a real set was seen this session and it is now gone). The
    // retry-budget path below accepts an empty without that backing and must
    // stay non-durable, or the poisoned-device self-heal is lost.
    let confirmed = remoteArray.length > 0 || queryState.config.remoteSeen === true;
    if (remoteArray.length === 0 && !queryState.config.remoteSeen) {
      const serverRowCount = opts?.serverRowCount;
      const knownEmpty = serverRowCount === 0;
      confirmed = knownEmpty;
      if (!knownEmpty) {
        // Unknown row count still gets a bounded escape hatch, so a server that
        // cannot report one never strands a device on a durable seed forever.
        const emptyReads = (queryState.config.emptyReads ?? 0) + 1;
        queryState.config.emptyReads = emptyReads;
        const exhausted =
          serverRowCount === null || serverRowCount === undefined
            ? emptyReads >= EMPTY_MEMBERSHIP_CONFIRMATIONS
            : false; // a positive row count is never "confirmed empty"
        if (!exhausted) {
          this.logger.debug(
            {
              hash,
              emptyReads,
              serverRowCount,
              Category: 'sp00ky-client::DataModule::updateQueryRemoteArray',
            },
            'Ignoring empty membership: the server still reports rows for this query'
          );
          return;
        }
      }
    }

    const epoch = this.local.epoch;
    queryState.config.remoteArray = remoteArray;
    // The single point where authoritative membership arrives (registration and
    // the `_00_list_ref` poll both land here), so it is where "we now know the
    // membership" is latched and where the durable mirror is written.
    queryState.config.membershipKnown = true;
    if (remoteArray.length > 0) {
      // Earns the right to believe a later empty set — that transition is a
      // genuine removal and must be honoured, or removed rows resurrect.
      queryState.config.remoteSeen = true;
      queryState.config.emptyReads = 0;
    }
    if (queryState.config.membershipKey) {
      await this.writeWindowMembership(queryState.config.membershipKey, remoteArray, confirmed);
    }
    try {
      await this.local.query(
        surql.seal(surql.updateSet('id', ['remoteArray'])),
        {
          id: queryState.config.id,
          remoteArray,
        },
        { epoch }
      );
    } catch (err) {
      if (err instanceof StaleEpochError) return;
      throw err;
    }
  }

  /**
   * Cancel every armed timer ahead of a local-bucket switch: stream-update
   * debounce timers (their pending updates carry the OLD bucket's id-sets) and
   * per-query TTL heartbeats (they'd refresh the previous user's remote
   * `_00_query` rows under the new session). The rebind re-arms heartbeats.
   */
  quiesce(): void {
    for (const timer of this.debounceTimers.values()) {
      clearTimeout(timer);
    }
    this.debounceTimers.clear();
    this.pendingStreamUpdates.clear();
    this.fetchDepth.clear();
    for (const queryState of this.activeQueries.values()) {
      if (queryState.ttlTimer) {
        clearTimeout(queryState.ttlTimer);
        queryState.ttlTimer = null;
      }
    }
  }

  /**
   * Re-home every active query in a freshly-opened bucket, KEEPING its hash —
   * `useQuery` subscriptions are keyed by hash and don't re-register on auth
   * changes, so the hooks must stay attached. Per query:
   *   1. reset the sync arrays + hydration flag and drop the previous user's
   *      records, notifying subscribers with the new-bucket materialization
   *      (usually empty) so their rows leave the UI immediately;
   *   2. recreate the `_00_query` row in the new bucket;
   *   3. re-register the SSP view on the (fresh, post-reset) processor — this
   *      also rebinds the view to the NEW `$auth` context;
   *   4. restart the TTL heartbeat.
   * Returns the hashes so the caller can enqueue remote re-registration, which
   * refills records from the server via the normal register→sync→notify path.
   */
  async rebindAfterBucketSwitch(): Promise<QueryHash[]> {
    const hashes: QueryHash[] = [];
    for (const [hash, queryState] of this.activeQueries.entries()) {
      const config = queryState.config;
      config.localArray = [];
      config.remoteArray = [];
      config.subqueryRemoteArray = undefined;
      // Membership belongs to the bucket we just left. Re-seed from the NEW
      // bucket's durable row if it has one (a returning user), otherwise mark it
      // unknown so the first paint falls back to a scan instead of rendering an
      // empty list until the server answers.
      config.membershipKnown = false;
      // The new bucket's server sets have not been seen yet — an empty read
      // must be re-confirmed there too.
      config.remoteSeen = false;
      config.emptyReads = 0;
      if (config.membershipKey) {
        const durable = await this.getWindowMembership(config.membershipKey);
        // Same rule as the cold-start read: a non-empty row, or an empty one
        // the server confirmed, is membership; an unmarked empty row is not.
        if (durable && (durable.ids.length > 0 || durable.confirmed)) {
          config.remoteArray = durable.ids;
          config.membershipKnown = true;
        }
      }
      queryState.hydrated = false;
      queryState.syncNotified = false;
      queryState.records = [];
      // Via setQueryStatus (not a bare assignment) so status observers see the
      // flip back to a loading state.
      this.setQueryStatus(hash, 'fetching');

      try {
        await withRetry(this.logger, () =>
          this.local.query<[QueryConfigRecord]>(surql.seal(surql.create('id', 'data')), {
            id: config.id,
            data: {
              surql: config.surql,
              params: config.params,
              localArray: [],
              remoteArray: [],
              lastActiveAt: new Date(),
              createdAt: new Date(),
              ttl: config.ttl,
              tableName: config.tableName,
              updateCount: queryState.updateCount,
              rowCount: 0,
              errorCount: queryState.errorCount,
            },
          })
        );

        const { localArray } = this.cache.registerQuery({
          queryHash: hash,
          surql: config.surql,
          params: config.params,
          ttl: new Duration(config.ttl),
          lastActiveAt: new Date(),
        });
        config.localArray = localArray;
        await this.local.query(surql.seal(surql.updateSet('id', ['localArray', 'rowCount'])), {
          id: config.id,
          localArray,
          rowCount: localArray.length,
        });
      } catch (err) {
        this.logger.error(
          { err, hash, Category: 'sp00ky-client::DataModule::rebindAfterBucketSwitch' },
          'Failed to rebind query after bucket switch; remote re-registration will retry'
        );
      }

      // Notify AFTER the SSP re-registration so a subscriber that re-reads
      // synchronously sees consistent (empty) state.
      const subscribers = this.subscriptions.get(hash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }
      this.startTTLHeartbeat(queryState, hash);
      hashes.push(hash);
    }
    return hashes;
  }

  /**
   * Called after a query's initial sync completes.
   * Ensures subscribers are notified even if no stream updates fired (e.g. empty result set).
   */
  async notifyQuerySynced(queryHash: string): Promise<void> {
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) return;
    const epoch = this.local.epoch;

    // Re-query local DB for latest data (windowed queries materialize from the
    // list_ref window so they resolve even if the in-browser SSP never emits —
    // it can't compute a high offset whose preceding rows aren't resident).
    const newRecords = await this.materializeRecords(queryState);
    // Bucket switched while we materialized: these rows mix old-bucket reads
    // with new-bucket state — drop them; the rebind/re-registration re-emits.
    if (epoch !== this.local.epoch) return;
    const changed = JSON.stringify(queryState.records) !== JSON.stringify(newRecords);
    queryState.records = newRecords;

    // Notify if data changed OR if this registration lifetime hasn't emitted a
    // post-sync notification yet. The latter handles "query truly has no
    // results" so the UI can stop loading — gated on the in-memory
    // `syncNotified` flag rather than `updateCount === 0`, because updateCount
    // is PERSISTED across deregister/re-register: a re-registered empty window
    // (updateCount > 0, records unchanged) would otherwise never emit and its
    // subscribers would show a loading state forever.
    if (changed || !queryState.syncNotified) {
      queryState.syncNotified = true;
      queryState.updateCount++;
      queryState.lastUpdatedAt = Date.now();
      const subscribers = this.subscriptions.get(queryHash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }
    }
  }

  // ====================      RUN JOBS       ====================

  async run<B extends BackendNames<S>, R extends BackendRoutes<S, B>>(
    backend: B,
    path: R,
    data: RoutePayload<S, B, R>,
    options?: RunOptions
  ): Promise<void> {
    const { tableName, record } = this.buildJobRecord(backend, path, data, options);
    const recordId = `${tableName}:${generateId()}`;
    await this.create(recordId, record);
  }

  /**
   * Build the outbox job record + resolve its table for a backend route.
   *
   * Every job is a single execution. Recurring work is declared server-side
   * (`schedules:` in sp00ky.yml) and the scheduler creates a fresh row per cycle,
   * so nothing here needs to know about schedules.
   */
  private buildJobRecord<B extends BackendNames<S>, R extends BackendRoutes<S, B>>(
    backend: B,
    path: R,
    data: RoutePayload<S, B, R>,
    options?: RunOptions
  ): { tableName: string; record: Record<string, unknown> } {
    const route = this.schema.backends?.[backend]?.routes?.[path];
    if (!route) {
      throw new Error(`Route ${backend}.${path} not found`);
    }

    const tableName = this.schema.backends?.[backend]?.outboxTable;
    if (!tableName) {
      throw new Error(`Outbox table for backend ${backend} not found`);
    }

    const payload: Record<string, unknown> = {};
    for (const argName of Object.keys(route.args)) {
      const arg = route.args[argName];
      if ((data as Record<string, unknown>)[argName] === undefined && arg.optional === false) {
        throw new Error(`Missing required argument ${argName}`);
      }
      payload[argName] = (data as Record<string, unknown>)[argName];
    }

    const record: Record<string, unknown> = {
      path,
      payload: JSON.stringify(payload),
      // Set explicitly: the schema's DEFAULT ALWAYS "pending" is server-side
      // only. An optimistic local create without it surfaces with status
      // undefined, so in-flight indicators keyed on pending/processing stay
      // off until the first server echo (seconds on a delayed job).
      status: 'pending',
      max_retries: options?.max_retries ?? 3,
      retry_strategy: options?.retry_strategy ?? 'linear',
    };

    if (options?.timeout != null) {
      record.timeout = options.timeout;
    }

    if (options?.delay != null) {
      record.delay = options.delay;
    }

    if (options?.assignedTo) {
      record.assigned_to = options.assignedTo;
    }

    return { tableName, record };
  }

  // ==================== MUTATION MANAGEMENT ====================

  /**
   * Create a new record
   */
  async create<T extends Record<string, unknown>>(id: string, data: T): Promise<T> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const params = parseParams(tableSchema.columns, data);
    const mutationId = parseRecordIdString(mintMutationId(this.tabId));

    const dataKeys = Object.keys(params).map((key) => ({ key, variable: `data_${key}` }));
    const prefixedParams = Object.fromEntries(
      dataKeys.map(({ key, variable }) => [variable, params[key]])
    );
    const query = surql.seal<T>(
      surql.tx([
        surql.createSet('id', dataKeys),
        surql.createMutation('create', 'mid', 'id', 'data'),
      ]),
      { resultIndex: 0 }
    );

    const target = await withRetry(this.logger, () =>
      this.local.execute(query, {
        id: rid,
        mid: mutationId,
        // The record itself is written with per-key `data_<field>` vars
        // (`createSet`), but the OUTBOX row needs the whole payload in one
        // `data` field so the mutation can be replayed later. Without this the
        // row is written with `data` unbound: the create becomes unsendable and
        // blocks the queue behind it.
        data: params,
        ...prefixedParams,
      })
    );

    // The local tx has committed: the row and its outbox entry exist. The
    // cached pending-id sets no longer describe the outbox, so drop them HERE,
    // before the ingest below fans out to every view's materialization (which
    // is exactly when they are read).
    this.invalidatePendingIds();

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache (which handles DBSP ingestion). Best-effort, like
    // `delete()`: the local write is durable and the outbox row exists, so a
    // throw from the in-browser ingest must not abort the rest of this method -
    // it used to, and then the mutation was never handed to sync, the row sat
    // in the outbox until the next leader promotion re-scanned it, and the
    // caller's promise rejected after the write had already happened.
    try {
      await this.cache.save(
        {
          table: tableName,
          op: 'CREATE',
          record: parsedRecord,
          version: 1,
        },
        true
      );
    } catch (err) {
      this.logger.error(
        { err, id, Category: 'sp00ky-client::DataModule::create' },
        'SSP create-ingest failed; the row is written and queued, but views only see it once membership arrives'
      );
    }

    // Emit mutation event for sync
    const mutationEvent: CreateEvent = {
      type: 'create',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
      tableName,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::create' }, 'Record created');

    return target;
  }

  /**
   * Update an existing record
   */
  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>,
    options?: UpdateOptions
  ): Promise<T> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const params = parseParams(tableSchema.columns, data);
    const mutationId = parseRecordIdString(mintMutationId(this.tabId));

    // Note: CRDT state is pushed directly to the _00_crdt table by CrdtField.pushToRemote(),
    // NOT through the record update pipeline. This keeps the record data clean.

    // Capture current record state before mutation for rollback support
    const [beforeRecord] = await withRetry(this.logger, () =>
      this.local.query<[Record<string, any>]>('SELECT * FROM ONLY $id', { id: rid })
    );

    const query = surql.seal<{ target: T }>(
      surql.tx([
        surql.updateSet('id', [{ statement: '_00_rv += 1' }]),
        surql.let('updated', surql.updateMerge('id', 'data')),
        surql.createMutation('update', 'mid', 'id', 'data'),
        surql.returnObject([{ key: 'target', variable: 'updated' }]),
      ])
    );

    const { target } = await withRetry(this.logger, () =>
      this.local.execute(query, {
        id: rid,
        mid: mutationId,
        data: params,
      })
    );

    // Build a partial record with only the fields the user actually changed
    // This avoids overwriting rich relation objects (e.g. author: {id, name, ...})
    // with flat RecordIds from the UPDATE...MERGE result
    const updatedFields: Record<string, any> = { id: target.id };
    for (const key of Object.keys(data)) {
      if (key in target) {
        updatedFields[key] = (target as Record<string, any>)[key];
      }
    }
    if ('_00_rv' in (target as Record<string, any>)) {
      updatedFields._00_rv = (target as Record<string, any>)._00_rv;
    }
    this.replaceRecordInQueries(updatedFields);

    // Committed: see create() for why this precedes the ingest.
    this.invalidatePendingIds();

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache. Best-effort for the same reason as in create().
    try {
      await this.cache.save(
        {
          table: table,
          op: 'UPDATE',
          record: parsedRecord,
          version: target._00_rv as number,
        },
        true
      );
    } catch (err) {
      this.logger.error(
        { err, id, Category: 'sp00ky-client::DataModule::update' },
        'SSP update-ingest failed; the row is written and queued'
      );
    }

    const pushEventOptions = parseUpdateOptions(id, data, options);

    // Emit mutation event
    const mutationEvent: UpdateEvent = {
      type: 'update',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
      beforeRecord: beforeRecord || undefined,
      options: pushEventOptions,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::update' }, 'Record updated');

    return target;
  }

  /**
   * Delete a record
   */
  async delete(table: string, id: string): Promise<void> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const mutationId = parseRecordIdString(mintMutationId(this.tabId));

    // Fetch the record before deleting so DBSP can match it against query predicates
    const [beforeRecords] = await this.local.query<[Record<string, any>[]]>(
      'SELECT * FROM ONLY $id',
      { id: rid }
    );
    const beforeRecord = beforeRecords ?? {};

    const query = surql.seal<void>(
      surql.tx([surql.delete('id'), surql.createMutation('delete', 'mid', 'id')])
    );

    await withRetry(this.logger, () => this.local.execute(query, { id: rid, mid: mutationId }));

    // Committed: the outbox row exists, drop the cached pending-id sets before
    // the re-materialize below reads them (see create()).
    this.invalidatePendingIds();

    // The local DELETE has now committed. Everything below must reflect that in
    // active live queries — so the deleted row disappears optimistically without
    // a reload — even if the optimistic SSP-view ingest below fails. Previously a
    // throw from `cache.delete` (the WASM ingest) aborted `delete()` after the
    // commit, so the manual notify loop never ran and the row lingered on screen
    // until reload. Ingesting the delete into the in-browser SSP view is
    // best-effort: the manual re-materialize reads the local DB (which already
    // excludes the row), so the result is correct regardless.
    try {
      await this.cache.delete(table, id, true, beforeRecord);
    } catch (err) {
      this.logger.error(
        { err, id, Category: 'sp00ky-client::DataModule::delete' },
        'SSP delete-ingest failed; relying on query re-materialize to reflect the delete'
      );
    }

    // DBSP may not emit view updates for DELETE ops — manually notify all queries
    // that reference this table.
    await this.notifyTableQueries(tableName);

    // Emit mutation event
    const mutationEvent: DeleteEvent = {
      type: 'delete',
      mutation_id: mutationId,
      record_id: rid,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::delete' }, 'Record deleted');
  }

  // ==================== ROLLBACK METHODS ====================

  /**
   * Rollback a failed optimistic create by deleting the record locally
   */
  async rollbackCreate(recordId: RecordId, tableName: string): Promise<void> {
    const id = encodeRecordId(recordId);

    try {
      await withRetry(this.logger, () => this.local.query('DELETE $id', { id: recordId }));
      await this.cache.delete(tableName, id, true);
      this.removeRecordFromQueries(recordId);

      this.logger.info(
        { id, tableName, Category: 'sp00ky-client::DataModule::rollbackCreate' },
        'Rolled back optimistic create'
      );
    } catch (err) {
      this.logger.error(
        { err, id, tableName, Category: 'sp00ky-client::DataModule::rollbackCreate' },
        'Failed to rollback create'
      );
    }
  }

  /**
   * Rollback a failed optimistic update by restoring the previous record state
   */
  async rollbackUpdate(
    recordId: RecordId,
    tableName: string,
    beforeRecord: Record<string, unknown>
  ): Promise<void> {
    const id = encodeRecordId(recordId);

    try {
      const { id: _recordId, ...content } = beforeRecord;
      await withRetry(this.logger, () =>
        this.local.query(surql.seal(surql.upsert('id', 'content')), {
          id: recordId,
          content,
        })
      );

      const tableSchema = this.schema.tables.find((t) => t.name === tableName);
      const parsedRecord = tableSchema
        ? (parseParams(tableSchema.columns, beforeRecord) as RecordWithId)
        : (beforeRecord as RecordWithId);

      await this.cache.save(
        {
          table: tableName,
          op: 'UPDATE',
          record: parsedRecord,
          version: (beforeRecord._00_rv as number) || 1,
        },
        true
      );

      // Replace in active queries for immediate UI update
      await this.replaceRecordInQueries(beforeRecord);

      this.logger.info(
        { id, tableName, Category: 'sp00ky-client::DataModule::rollbackUpdate' },
        'Rolled back optimistic update'
      );
    } catch (err) {
      this.logger.error(
        { err, id, tableName, Category: 'sp00ky-client::DataModule::rollbackUpdate' },
        'Failed to rollback update'
      );
    }
  }

  /**
   * Force a re-materialize + notify of every active query on `tableName`.
   * Used after a DELETE landed in the local store (this tab's own, or one
   * relayed from another tab): the SSP may not emit a view update for a
   * DELETE ingest, and the re-materialize reads the store, which already
   * excludes the row. Each query is isolated so one failing re-materialize
   * can't stop the others.
   */
  async notifyTableQueries(tableName: string): Promise<void> {
    for (const [queryHash, queryState] of this.activeQueries) {
      if (queryState.config.tableName === tableName) {
        try {
          await this.notifyQuerySynced(queryHash);
        } catch (err) {
          this.logger.error(
            { err, queryHash, tableName, Category: 'sp00ky-client::DataModule::notifyTableQueries' },
            'notifyQuerySynced failed after delete'
          );
        }
      }
    }
  }

  /**
   * Remove a record from all active query states and notify subscribers
   */
  private removeRecordFromQueries(recordId: RecordId): void {
    const encodedId = encodeRecordId(recordId);

    for (const [queryHash, queryState] of this.activeQueries.entries()) {
      const index = queryState.records.findIndex((r) => {
        const rId = r.id instanceof RecordId ? encodeRecordId(r.id) : String(r.id);
        return rId === encodedId;
      });

      if (index !== -1) {
        queryState.records.splice(index, 1);
        const subscribers = this.subscriptions.get(queryHash);
        if (subscribers) {
          for (const callback of subscribers) {
            callback(queryState.records);
          }
        }
      }
    }
  }

  // ==================== PRIVATE HELPERS ====================

  private async createAndRegisterQuery<T extends TableNames<S>>(
    hash: QueryHash,
    recordId: RecordId,
    surqlString: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    tableName: T,
    plan?: QueryPlan,
    membershipKey?: string
  ): Promise<QueryHash> {
    const queryState = await this.createNewQuery<T>({
      recordId,
      surql: surqlString,
      params,
      ttl,
      tableName,
      plan,
      membershipKey,
    });

    const t0 = performance.now();
    const { localArray, registrationTimings } = this.cache.registerQuery({
      queryHash: hash,
      surql: surqlString,
      params,
      ttl: new Duration(ttl),
      lastActiveAt: new Date(),
    });
    const registrationTime = performance.now() - t0;

    // Record the one-shot SSP registration timings (parse/plan/snapshot from the
    // WASM binding) + the register_view wall time for DevTools.
    queryState.registrationTimings = {
      parseMs: registrationTimings?.parseMs ?? null,
      planMs: registrationTimings?.planMs ?? null,
      snapshotMs: registrationTimings?.snapshotMs ?? null,
      wallMs: registrationTime,
    };

    await withRetry(this.logger, () =>
      this.local.query(
        surql.seal(surql.updateSet('id', ['localArray', 'registrationTime', 'rowCount'])),
        {
          id: recordId,
          localArray,
          registrationTime,
          rowCount: localArray.length,
        }
      )
    );

    // Windowed (`START n`) queries skipped the raw initial load in
    // createNewQuery (O(offset) + wrong rows for sparse windows). Seed the
    // initial rows now from the SSP's window id-set (`localArray`) via the same
    // window-materialization path the stream updates use — O(window), and the
    // ids are already the correct window — so the first paint isn't empty while
    // the remote `_00_list_ref` syncs in.
    const windowMat = buildWindowMaterialization(surqlString);
    if (windowMat && localArray.length > 0) {
      try {
        const winIds = localArray.map(([id]) => parseRecordIdString(id));
        if (plan) {
          const winPlan = buildWindowMaterializationPlan(plan, winIds) ?? { ...plan, ids: winIds };
          queryState.records = await this.local.select(winPlan, params);
        } else {
          const [seeded] = await this.local.query<[Record<string, any>[]]>(windowMat.query, {
            ...params,
            __win: winIds,
          });
          queryState.records = seeded || [];
        }
      } catch (err) {
        this.logger.warn(
          { err, hash, Category: 'sp00ky-client::DataModule::createAndRegisterQuery' },
          'Failed to seed windowed initial records from localArray'
        );
      }
    }

    this.activeQueries.set(hash, queryState);
    this.startTTLHeartbeat(queryState, hash);
    this.logger.debug(
      {
        hash,
        tableName,
        recordCount: queryState.records.length,
        Category: 'sp00ky-client::DataModule::query',
      },
      'Query registered'
    );

    return hash;
  }

  private async createNewQuery<T extends TableNames<S>>({
    recordId,
    surql: surqlString,
    params,
    ttl,
    tableName,
    plan,
    membershipKey,
  }: {
    recordId: RecordId;
    surql: string;
    params: Record<string, any>;
    ttl: QueryTimeToLive;
    tableName: T;
    plan?: QueryPlan;
    membershipKey?: string;
  }): Promise<QueryState> {
    // `_00_*` meta tables (feature flags, app releases) are framework-owned:
    // they exist in the client db schema by construction but are never part of
    // the app's generated `schema.tables`, so give them an empty column map
    // instead of the not-found error (which silently broke every meta-table
    // live query, e.g. feature flags never updating on this path).
    const tableSchema =
      this.schema.tables.find((t) => t.name === tableName) ??
      (String(tableName).startsWith('_00_')
        ? ({ name: tableName, columns: {} } as any)
        : undefined);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    let [configRecord] = await withRetry(this.logger, () =>
      this.local.query<[QueryConfigRecord]>('SELECT * FROM ONLY $id', {
        id: recordId,
      })
    );

    if (!configRecord) {
      const [createdRecord] = await withRetry(this.logger, () =>
        this.local.query<[QueryConfigRecord]>(surql.seal(surql.create('id', 'data')), {
          id: recordId,
          data: {
            surql: surqlString,
            params: params,
            localArray: [],
            remoteArray: [],
            lastActiveAt: new Date(),
            createdAt: new Date(),
            ttl,
            tableName,
            updateCount: 0,
            rowCount: 0,
            errorCount: 0,
          },
        })
      );
      configRecord = createdRecord;
    }

    const config: QueryConfig = {
      ...configRecord,
      id: recordId,
      // In-memory only — carries the engine-neutral plan so non-SurrealQL local
      // engines materialize via `select(plan)` instead of parsing `surql`.
      plan,
      // The CALLER's params, not the ones read back out of the local store: the
      // store returns a RecordId as a plain `'table:id'` string, and
      // `parseQueryParams` can only turn it back into a RecordId for a column
      // the schema marks `recordId`. A union column (`string | record<x>`)
      // codegens as a plain string, so its param stayed a string and the view
      // compared a string against a record and matched nothing. The record id is
      // the same query either way - `recordId` is the hash of surql + vars - so
      // the in-memory value is the same one, with its types intact.
      params: parseQueryParams(tableSchema.columns, params ?? configRecord.params),
      membershipKey,
    };

    // The `_00_query` row we just read is session-salted, so on a reload it is
    // always a fresh row with empty arrays. Recover the last authoritative
    // membership from the durable `_00_window` row instead — this is what stops a
    // removed row reappearing after a reload, and it works with no network.
    if (membershipKey && !config.remoteArray?.length) {
      const durable = await this.getWindowMembership(membershipKey);
      // An empty durable row is trusted only when it carries the `confirmed`
      // marker, i.e. the server itself reported the query empty. Without it an
      // empty row cannot be told apart from one written before this device ever
      // saw a real id-set (or by the pre-`ea56f50e` client that mirrored
      // unflushed reads), and treating it as known would paint an empty list
      // with no scan fallback. A confirmed empty is the opposite case: the
      // server said "no rows", so a reload must stay empty rather than re-admit
      // every cached body until the next poll blanks it again.
      if (durable && (durable.ids.length > 0 || durable.confirmed)) {
        config.remoteArray = durable.ids;
        config.membershipKnown = true;
      }
    } else if (config.remoteArray?.length) {
      config.membershipKnown = true;
    }

    let records: Record<string, any>[] = [];
    // Windowed (`START n`) queries: do NOT seed from the raw surql here. Running
    // `… LIMIT n START m` against the shared local store is O(m) — it sorts and
    // skips m rows on every window open — AND returns the wrong rows for sparse
    // windows (the reason `buildWindowMaterialization` exists). Those windows are
    // seeded from the SSP `localArray` in `createAndRegisterQuery` instead.
    //
    // With known membership the same reasoning applies to EVERY query, windowed
    // or not: the predicate scan below would re-admit rows the server has already
    // dropped from the window (their bodies are still cached and still match the
    // WHERE). Seed from membership instead.
    if (config.membershipKnown) {
      try {
        records = await this.materializeFromConfig(config);
      } catch (err) {
        this.logger.warn(
          { err, Category: 'sp00ky-client::DataModule::createNewQuery' },
          'Failed to seed records from durable membership'
        );
      }
    } else if (buildWindowMaterialization(surqlString) === null) {
      try {
        // Prefer the engine-neutral plan (required for non-SurrealQL engines);
        // fall back to running the raw surql on the SurrealDB engine.
        const result = plan
          ? await this.local.select(plan, params)
          : (await this.local.query<[Record<string, any>[]]>(surqlString, params))[0];
        records = result || [];
      } catch (err) {
        this.logger.warn(
          { err, Category: 'sp00ky-client::DataModule::createNewQuery' },
          'Failed to load initial cached records'
        );
      }
    }

    // Persisted counters survive a restart even though the rolling
    // sample window is rebuilt from scratch in memory.
    const persistedUpdateCount =
      typeof (configRecord as any)?.updateCount === 'number'
        ? (configRecord as any).updateCount
        : 0;
    const persistedErrorCount =
      typeof (configRecord as any)?.errorCount === 'number' ? (configRecord as any).errorCount : 0;

    return {
      config,
      records,
      ttlTimer: null,
      ttlDurationMs: parseDuration(ttl),
      updateCount: persistedUpdateCount,
      lastUpdatedAt: null,
      materializationSamples: [],
      lastIngestLatencyMs: null,
      errorCount: persistedErrorCount,
      // Born `fetching`, not `idle`: every cold registration is followed by a
      // `register` down-event whose lifecycle (Sp00kySync.registerQuery) resolves
      // the status to `idle` once the initial sync completed. Starting idle left
      // a gap where a fresh windowed query looked settled while still empty.
      status: 'fetching',
      phaseSamples: {},
      phaseLast: {},
      registrationTimings: { parseMs: null, planMs: null, snapshotMs: null, wallMs: null },
    };
  }

  private async calculateHash(data: any): Promise<string> {
    // sessionId is part of the hash so the same logical query from two
    // sessions (e.g. two browser tabs of the same user) lands on different
    // `_00_query` rows and doesn't fight over a shared one.
    return this.sha256(JSON.stringify({ ...data, sessionId: this.sessionId }));
  }

  /**
   * Session-independent counterpart of {@link calculateHash}: the key for a
   * query's durable `_00_window` membership row.
   *
   * Deliberately the SAME inputs minus the `session::id()` salt, so the two keys
   * can never drift apart. The salt is right for `_00_query` (two tabs must not
   * fight over one row) and wrong for membership, which has to be recognizable
   * after a reload — a reload mints a new session id, and offline the salt is
   * `''`, so a salted key can never match what the previous session wrote.
   */
  private async calculateMembershipKey(data: any): Promise<string> {
    return this.sha256(JSON.stringify(data));
  }

  private async sha256(content: string): Promise<string> {
    const msgBuffer = new TextEncoder().encode(content);
    const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
  }

  private startTTLHeartbeat(queryState: QueryState, hash: QueryHash): void {
    if (queryState.ttlTimer) return;

    // Half the TTL, not 0.9 of it, so ONE failed beat is survivable.
    //
    // The timer re-arms after each beat, so expiry sits a full TTL after the
    // last SUCCESSFUL refresh. At 0.9 a beat that fails — the SSP restarting,
    // a reconnect, a slow database — left 10% of the TTL before the sweep
    // deleted the row AND its `_00_list_ref` edges, with no second attempt in
    // between. On a 10m TTL that is 60 seconds of slack against an SSP
    // bootstrap that takes ~2 minutes on a large tenant, so the view lost a
    // race it could not win.
    //
    // The user-visible form is content vanishing and coming back: the client
    // is LIVE on those edges, so it sees the sweep's deletes immediately, and
    // only restores them once its heartbeat notices the row is gone and
    // re-registers. Reported as chat messages disappearing for 10+ seconds.
    //
    // At 0.5 a single failure still leaves another attempt and half the TTL.
    // The cost is one extra beat per view per TTL, which is a single
    // `UPDATE ... SET lastActiveAt`.
    const heartbeatTime = Math.floor(queryState.ttlDurationMs * TTL_HEARTBEAT_FRACTION);

    queryState.ttlTimer = setTimeout(() => {
      queryState.ttlTimer = null;
      // Only keep the remote query alive while something is actually watching
      // it. With the server now sweeping ALL expired views (not just in-circuit
      // ones), an un-refreshed query WOULD be swept after its TTL — so a live
      // subscriber must heartbeat. An abandoned query (no subscribers) is left
      // to expire and get swept; its local view was already torn down at the
      // last unsubscribe, so we just stop the timer here.
      const subscriberCount = this.subscriptions.get(hash)?.size ?? 0;
      if (subscriberCount === 0) {
        this.logger.debug(
          { hash, Category: 'sp00ky-client::DataModule::startTTLHeartbeat' },
          'TTL heartbeat: no subscribers, stopping'
        );
        return;
      }
      this.onHeartbeat?.(hash);
      this.logger.debug(
        {
          hash,
          id: encodeRecordId(queryState.config.id),
          Category: 'sp00ky-client::DataModule::startTTLHeartbeat',
        },
        'TTL heartbeat sent'
      );
      this.startTTLHeartbeat(queryState, hash);
    }, heartbeatTime);
  }

  private async replaceRecordInQueries(record: Record<string, any>): Promise<void> {
    for (const [queryHash, queryState] of this.activeQueries.entries()) {
      const index = queryState.records.findIndex((r) => r.id === record.id);
      if (index !== -1) {
        queryState.records[index] = { ...queryState.records[index], ...record };
        // Notify subscribers so UI updates immediately
        const subscribers = this.subscriptions.get(queryHash);
        if (subscribers) {
          for (const callback of subscribers) {
            callback(queryState.records);
          }
        }
      }
    }
  }
}

// ==================== HELPER FUNCTIONS ====================

/**
 * Parse update options to generate push event options
 */
export function parseUpdateOptions(
  id: string,
  data: any,
  options?: UpdateOptions
): PushEventOptions {
  let pushEventOptions: PushEventOptions = {};
  if (options?.debounced) {
    const delay = options.debounced !== true ? (options.debounced?.delay ?? 200) : 200;
    const keyType = options.debounced !== true ? (options.debounced?.key ?? id) : id;
    const key =
      keyType === 'recordId_x_fields' ? `${id}::${Object.keys(data).toSorted().join('#')}` : id;

    pushEventOptions = {
      debounced: {
        delay,
        key,
      },
    };
  }
  return pushEventOptions;
}
