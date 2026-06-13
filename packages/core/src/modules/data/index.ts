import { RecordId, Duration } from 'surrealdb';
import type {
  SchemaStructure,
  TableNames,
  BackendNames,
  BackendRoutes,
  RoutePayload,
} from '@spooky-sync/query-builder';
import type { LocalDatabaseService } from '../../services/database/index';
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
  RunOptions} from '../../types';
import { MATERIALIZATION_SAMPLE_WINDOW } from '../../types';
import {
  parseRecordIdString,
  extractIdPart,
  encodeRecordId,
  parseDuration,
  withRetry,
  surql,
  parseParams,
  cleanRecord,
  extractTablePart,
  generateId,
} from '../../utils/index';
import type { CreateEvent, DeleteEvent, UpdateEvent } from '../sync/index';
import type { PushEventOptions } from '../../events/index';
import { buildWindowMaterialization } from './window-query';

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
export class DataModule<S extends SchemaStructure> {
  private activeQueries: Map<QueryHash, QueryState> = new Map();
  private pendingQueries: Map<QueryHash, Promise<QueryHash>> = new Map();
  private subscriptions: Map<QueryHash, Set<QueryUpdateCallback>> = new Map();
  private statusSubscriptions: Map<QueryHash, Set<QueryStatusCallback>> = new Map();
  private mutationCallbacks: Set<MutationCallback> = new Set();
  private debounceTimers: Map<QueryHash, NodeJS.Timeout> = new Map();
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
    private local: LocalDatabaseService,
    private schema: S,
    logger: Logger,
    private streamDebounceTime: number = 100
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
    ttl: QueryTimeToLive
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
    const promise = this.createAndRegisterQuery<T>(hash, recordId, surqlString, params, ttl, tableName);
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
      await this.processStreamUpdate(update);
      return;
    }

    // Clear existing timer if any
    if (this.debounceTimers.has(queryHash)) {
      // oxlint-disable-next-line no-non-null-assertion -- guarded by .has() check above
      clearTimeout(this.debounceTimers.get(queryHash)!);
    }

    // Set new timer
    const timer = setTimeout(async () => {
      this.debounceTimers.delete(queryHash);
      await this.processStreamUpdate(update);
    }, this.streamDebounceTime);

    this.debounceTimers.set(queryHash, timer);
  }

  // Materialize a query's result rows from the local DB. For a windowed query
  // (`LIMIT n START m`, m>0) the original surql is NOT re-run — re-applying
  // `START m` against the shared local store would skip the window's own rows
  // (sparse windowing) and return nothing. Instead we select the window's
  // record id-set directly, preferring the server's `list_ref` (`remoteArray`,
  // authoritative) over the in-browser SSP's view (`sspArray`), and re-apply
  // the original ORDER BY for stable order.
  private async materializeRecords(
    queryState: QueryState,
    sspArray?: Array<[string, number]>
  ): Promise<Record<string, any>[]> {
    const t0 = performance.now();
    const windowMat = buildWindowMaterialization(queryState.config.surql);
    let records: Record<string, any>[];
    if (windowMat) {
      const win =
        (queryState.config.remoteArray?.length && queryState.config.remoteArray) ||
        (sspArray?.length && sspArray) ||
        queryState.config.localArray ||
        [];
      const winIds = win.map(([id]) => parseRecordIdString(id));
      const [rows] = await this.local.query<[Record<string, any>[]]>(windowMat.query, {
        ...queryState.config.params,
        __win: winIds,
      });
      records = rows || [];
    } else {
      const [rows] = await this.local.query<[Record<string, any>[]]>(
        queryState.config.surql,
        queryState.config.params
      );
      records = rows || [];
    }
    // Local SurrealDB record-fetch time → DevTools "localFetch" phase.
    this.recordPhase(queryState, 'localFetch', performance.now() - t0);
    return records;
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

    try {
      // Materialize the query's rows. For a windowed (offset) query, re-running
      // the original surql would re-apply `START n` against the shared local DB
      // and skip the window's rows entirely; instead select the SSP's
      // materialized window id-set (`localArray`) directly, re-applying the
      // original ORDER BY for stable display order. Non-offset queries keep the
      // normal re-query path.
      const newRecords = await this.materializeRecords(queryState, localArray);
      queryState.config.localArray = localArray;

      const prevJson = JSON.stringify(queryState.records);
      const newJson = JSON.stringify(newRecords);
      queryState.records = newRecords;
      const recordsChanged = prevJson !== newJson;

      // updateCount counts user-visible updates (matches the prior semantic),
      // while the materialization sample/percentiles already moved above for
      // every observed engine step.
      if (recordsChanged) {
        queryState.updateCount++;
      }

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
        }
      );

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
    return { p55: pick(0.55), p90: pick(0.90), p99: pick(0.99) };
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
          record: (tableSchema
            ? cleanRecord(tableSchema.columns, child)
            : child) as RecordWithId,
          version: (child._00_rv as number) || 1,
        });
      }
    }
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

    const tableName = queryState.config.tableName;
    const batch: CacheRecord[] = rows.map((record) => ({
      table: tableName,
      op: 'CREATE' as const,
      record,
      version: (record._00_rv as number) || 1,
    }));

    // Flicker-free related data on cold first paint: a `.related()` query's
    // rows arrive with their child rows EMBEDDED (the correlated subquery,
    // e.g. `(SELECT * FROM comment WHERE game=$parent.id) AS comments`). The
    // primary batch above persists only the parent table, so the immediate
    // `materializeRecords` below would re-run the correlated surql against a
    // local DB with no child rows and overwrite the embedded result with
    // empties. Extract those embedded children (any nesting depth) and
    // persist them as their own records so the re-materialization finds them.
    const seen = new Set<string>(rows.map((r) => encodeRecordId(r.id)));
    for (const record of rows) {
      this.collectEmbeddedChildren(record, batch, seen);
    }

    await this.cache.saveBatch(batch);

    // Prime remoteArray from the hydrated id+version pairs: `materializeRecords`
    // prefers it for windowed queries (correct window) and it feeds the version
    // dedup. Registration later overwrites it with the authoritative `_00_list_ref`.
    queryState.config.remoteArray = rows.map(
      (r) => [encodeRecordId(r.id), (r._00_rv as number) || 1] as [string, number]
    );

    queryState.records = await this.materializeRecords(queryState);
    const subscribers = this.subscriptions.get(hash);
    if (subscribers) {
      for (const cb of subscribers) cb(queryState.records);
    }
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
    queryState.config.localArray = localArray;
    await this.local.query(surql.seal(surql.updateSet('id', ['localArray'])), {
      id: queryState.config.id,
      localArray,
    });
  }

  async updateQueryRemoteArray(hash: string, remoteArray: RecordVersionArray): Promise<void> {
    const queryState = this.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn(
        { hash, Category: 'sp00ky-client::DataModule::updateQueryRemoteArray' },
        'Query to update remote array not found'
      );
      return;
    }
    queryState.config.remoteArray = remoteArray;
    await this.local.query(surql.seal(surql.updateSet('id', ['remoteArray'])), {
      id: queryState.config.id,
      remoteArray,
    });
  }

  /**
   * Called after a query's initial sync completes.
   * Ensures subscribers are notified even if no stream updates fired (e.g. empty result set).
   */
  async notifyQuerySynced(queryHash: string): Promise<void> {
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) return;

    // Re-query local DB for latest data (windowed queries materialize from the
    // list_ref window so they resolve even if the in-browser SSP never emits —
    // it can't compute a high offset whose preceding rows aren't resident).
    const newRecords = await this.materializeRecords(queryState);
    const changed = JSON.stringify(queryState.records) !== JSON.stringify(newRecords);
    queryState.records = newRecords;

    // Notify if data changed OR if this is the first sync (updateCount === 0)
    // The latter handles "query truly has no results" so UI can stop loading
    if (changed || queryState.updateCount === 0) {
      queryState.updateCount++;
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
      max_retries: options?.max_retries ?? 3,
      retry_strategy: options?.retry_strategy ?? 'linear',
    };

    if (options?.timeout != null) {
      record.timeout = options.timeout;
    }

    if (options?.assignedTo) {
      record.assigned_to = options.assignedTo;
    }

    const recordId = `${tableName}:${generateId()}`;
    await this.create(recordId, record);
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
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

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
        ...prefixedParams,
      })
    );

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache (which handles DBSP ingestion)
    await this.cache.save(
      {
        table: tableName,
        op: 'CREATE',
        record: parsedRecord,
        version: 1,
      },
      true
    );

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
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

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

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache
    await this.cache.save(
      {
        table: table,
        op: 'UPDATE',
        record: parsedRecord,
        version: target._00_rv as number,
      },
      true
    );

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
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

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
    // that reference this table. Each is isolated so one failing re-materialize
    // can't stop the others (or the sync emit below) from running.
    for (const [queryHash, queryState] of this.activeQueries) {
      if (queryState.config.tableName === tableName) {
        try {
          await this.notifyQuerySynced(queryHash);
        } catch (err) {
          this.logger.error(
            { err, queryHash, Category: 'sp00ky-client::DataModule::delete' },
            'notifyQuerySynced failed after delete'
          );
        }
      }
    }

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
      await withRetry(this.logger, () =>
        this.local.query('DELETE $id', { id: recordId })
      );
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
    tableName: T
  ): Promise<QueryHash> {
    const queryState = await this.createNewQuery<T>({
      recordId,
      surql: surqlString,
      params,
      ttl,
      tableName,
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
        const [seeded] = await this.local.query<[Record<string, any>[]]>(windowMat.query, {
          ...params,
          __win: winIds,
        });
        queryState.records = seeded || [];
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
  }: {
    recordId: RecordId;
    surql: string;
    params: Record<string, any>;
    ttl: QueryTimeToLive;
    tableName: T;
  }): Promise<QueryState> {
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
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
      params: parseParams(tableSchema.columns, configRecord.params),
    };

    let records: Record<string, any>[] = [];
    // Windowed (`START n`) queries: do NOT seed from the raw surql here. Running
    // `… LIMIT n START m` against the shared local store is O(m) — it sorts and
    // skips m rows on every window open — AND returns the wrong rows for sparse
    // windows (the reason `buildWindowMaterialization` exists). Those windows are
    // seeded from the SSP `localArray` in `createAndRegisterQuery` instead.
    if (buildWindowMaterialization(surqlString) === null) {
      try {
        const [result] = await this.local.query<[Record<string, any>[]]>(surqlString, params);
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
      typeof (configRecord as any)?.errorCount === 'number'
        ? (configRecord as any).errorCount
        : 0;

    return {
      config,
      records,
      ttlTimer: null,
      ttlDurationMs: parseDuration(ttl),
      updateCount: persistedUpdateCount,
      materializationSamples: [],
      lastIngestLatencyMs: null,
      errorCount: persistedErrorCount,
      status: 'idle',
      phaseSamples: {},
      phaseLast: {},
      registrationTimings: { parseMs: null, planMs: null, snapshotMs: null, wallMs: null },
    };
  }

  private async calculateHash(data: any): Promise<string> {
    // sessionId is part of the hash so the same logical query from two
    // sessions (e.g. two browser tabs of the same user) lands on different
    // `_00_query` rows and doesn't fight over a shared one.
    const content = JSON.stringify({ ...data, sessionId: this.sessionId });
    const msgBuffer = new TextEncoder().encode(content);
    const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
  }

  private startTTLHeartbeat(queryState: QueryState, hash: QueryHash): void {
    if (queryState.ttlTimer) return;

    const heartbeatTime = Math.floor(queryState.ttlDurationMs * 0.9);

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
