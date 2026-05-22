import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { RecordVersionArray } from '../../types';
import { createSyncEventSystem, SyncEventTypes, SyncQueueEventTypes } from './events/index';
import type { Logger } from '../../services/logger/index';
import type { DownEvent, UpEvent} from './queue/index';
import { DownQueue, UpQueue } from './queue/index';
import type { RecordId, Uuid } from 'surrealdb';
import {
  ArraySyncer,
  buildListRefSelect,
  createDiffFromDbOp,
  nextPollDelayMs,
  resolveListRefPollInterval,
} from './utils';
import { SyncEngine } from './engine';
import { SyncScheduler } from './scheduler';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { CacheModule } from '../cache/index';
import type { DataModule } from '../data/index';
import { encodeRecordId, extractTablePart, surql } from '../../utils/index';
import { DEFAULT_REF_MODE, listRefTableFor, RefMode } from '../ref-tables';

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
  public readonly refSyncIntervalMs: number;

  // Wall-clock timestamp (ms) of the most recent LIVE event delivered
  // through `handleRemoteListRefChange`. Read by `nextPollDelayMs` to
  // back the poll off when LIVE is healthy.
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
      this.handleRollback.bind(this)
    );
    this.refSyncIntervalMs = resolveListRefPollInterval(options?.refSyncIntervalMs);
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
        try {
          await this.pollListRefForActiveQueries();
        } finally {
          if (!this.listRefPollRunning) return;
          const next = nextPollDelayMs({
            now: Date.now(),
            lastLiveEventAt: this.lastLiveEventAt,
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

  private async pollListRefForActiveQueries(): Promise<void> {
    const hashes = this.dataModule.getActiveQueryHashes();
    if (hashes.length === 0) return;
    for (const hash of hashes) {
      try {
        await this.refetchListRefForQuery(hash);
      } catch (err) {
        this.logger.debug(
          { err: (err as Error)?.message ?? err, hash, Category: 'sp00ky-client::Sp00kySync::pollListRefForActiveQueries' },
          'Per-query list_ref poll failed'
        );
      }
    }
  }

  /**
   * Pull the upstream list_ref entries for `queryHash`, diff them
   * against the local `remoteArray` cache, sync any added/updated rows
   * through the SyncEngine, then persist the new remoteArray. This is
   * the same shape `createRemoteQuery` does for its initial fetch and
   * what `handleRemoteListRefChange` does per-LIVE-event — we reuse
   * it on a timer as a fallback for missed LIVE notifications.
   */
  private async refetchListRefForQuery(queryHash: string): Promise<void> {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) return;
    const listRefTbl = this.listRefTable();
    const [items] = await this.remote.query<[{ out: RecordId<string>; version: number }[]]>(
      buildListRefSelect(listRefTbl),
      { in: queryState.config.id }
    );
    if (!Array.isArray(items)) return;
    const fresh: RecordVersionArray = items.map((item) => [
      encodeRecordId(item.out),
      item.version,
    ]);
    // Update the cached remoteArray and run `syncQuery` inline so the
    // sync engine fetches any added/updated rows immediately (instead
    // of waiting for the next scheduler pass). `syncQuery` writes
    // through `cache.saveBatch`, which both UPSERTs the local DB row
    // and ingests it into the in-browser SSP — the SSP's stream
    // updates then run `processStreamUpdate`, which re-queries the
    // local DB and notifies subscribers. We skip an explicit
    // `notifyQuerySynced` because that path runs concurrently with
    // the stream-update path and can race (e.g. notify with stale
    // records).
    await this.dataModule.updateQueryRemoteArray(queryHash, fresh);
    try {
      await this.syncQuery(queryHash);
    } catch (err) {
      this.logger.info(
        { err: (err as Error)?.message ?? err, queryHash, Category: 'sp00ky-client::Sp00kySync::refetchListRefForQuery' },
        'syncQuery failed during poll'
      );
    }
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
    return listRefTableFor(this.refMode, this.dataModule.getCurrentUserId());
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
    // Any LIVE delivery is evidence that the feed is healthy — even
    // a DELETE we'll ignore below or a notification for an unknown
    // local query counts. `nextPollDelayMs` reads this to slow the
    // poll fallback while LIVE is doing its job.
    this.lastLiveEventAt = Date.now();

    if (action === 'DELETE') {
      this.logger.debug(
        {
          queryId: queryId.toString(),
          recordId: recordId.toString(),
          Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
        },
        'Ignoring DELETE on list_ref — should not happen'
      );
      return;
    }

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
    await this.syncEngine.syncRecords(diff);
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
    return this.syncEngine.syncRecords(diff);
  }

  /**
   * Enqueues a list of mutations (up events) to be sent to the remote.
   * @param mutations Array of UpEvents (create/update/delete) to enqueue.
   */
  public async enqueueMutation(mutations: UpEvent[]) {
    this.scheduler.enqueueMutation(mutations);
  }

  private async registerQuery(queryHash: string) {
    try {
      this.logger.debug(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'Register Query state'
      );
      await this.createRemoteQuery(queryHash);
      await this.syncQuery(queryHash);
      // Always notify after sync completes — handles empty result sets
      // where no stream updates fire but the UI needs to stop loading
      await this.dataModule.notifyQuerySynced(queryHash);
    } catch (e) {
      this.logger.error(
        { err: e, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'registerQuery error'
      );
      throw e;
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
  }

  private async heartbeatQuery(queryHash: string) {
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

  private async cleanupQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::cleanupQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    await this.remote.query(`DELETE $id`, {
      id: queryState.config.id,
    });
  }
}
