import { LocalDatabaseService, RemoteDatabaseService } from '../database/index.js';
import { MutationEventTypes } from '../mutation/index.js';
import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { QueryEventTypes } from '../query/events.js';
import { SyncQueueEventTypes, createSyncEventSystem, SyncEventTypes } from './events.js';
import { createLogger, Logger } from '../logger/index.js';
import {
  CleanupEvent,
  DownEvent,
  DownQueue,
  HeartbeatEvent,
  RegisterEvent,
  UpEvent,
  UpQueue,
} from './queue/index.js';
import { RecordId, Duration, Uuid } from 'surrealdb';
import { diffRecordVersionArray, ArraySyncer } from './utils.js';
import { parseRecordIdString, decodeFromSpooky } from '../utils/index.js';
import { TableModel } from '@spooky/query-builder';
import { SchemaStructure } from '@spooky/query-builder';
import { StreamProcessorService } from '../stream-processor/index.js';
// import { QueryManager } from '../query/index.js'; // REMOVED

export class SpookySync<S extends SchemaStructure> {
  private upQueue: UpQueue;
  private downQueue: DownQueue;
  private isInit: boolean = false;
  private isSyncingUp: boolean = false;
  private isSyncingDown: boolean = false;
  private relationshipMap: Map<string, Set<string>> = new Map();
  private logger: Logger;
  public events = createSyncEventSystem();

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }

  constructor(
    private schema: S,
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private streamProcessor: StreamProcessorService,
    private clientId: string,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'SpookySync' });
    this.upQueue = new UpQueue(this.local);
    this.downQueue = new DownQueue(this.local);
    this.buildRelationshipMap();
  }

  private buildRelationshipMap() {
    if (!this.schema?.relationships) return;
    for (const rel of this.schema.relationships) {
      if (!this.relationshipMap.has(rel.from)) {
        this.relationshipMap.set(rel.from, new Set());
      }
      this.relationshipMap.get(rel.from)?.add(rel.field);
    }
  }

  public async init() {
    if (this.isInit) throw new Error('SpookySync is already initialized');
    this.isInit = true;
    await this.initUpQueue();
    await this.initDownQueue();
    void this.syncUp();
    void this.syncUp();
    void this.syncDown();
    void this.startLiveQuery();
  }

  private async startLiveQuery() {
    this.logger.debug({ clientId: this.clientId }, 'Starting live query');
    // Ensure clientId is set in remote if needed, but SpookySync usually assumes auth is handled.
    // If we need to set the variable for the session:
    // await this.remote.getClient().query('LET $clientId = $id', { id: this.clientId });
    // Actually QueryManager did: await this.remote.getClient().set('_spooky_client_id', this.clientId);
    await this.remote.getClient().set('_spooky_client_id', this.clientId);

    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _spooky_incantation WHERE clientId = $clientId',
      {
        clientId: this.clientId,
      }
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug({ message }, 'Live update received');
      if (message.action === 'UPDATE' || message.action === 'CREATE') {
        // Look for array, fallback to tree for backward compatibility if needed, or null
        const { id, hash, array, tree } = message.value;
        // Use array if present, otherwise ignore (assuming complete switch)
        // Or should we support both during migration?
        // User request: "switch ... to a flat array of records"
        // Let's expect 'array' field.

        // Note: The backend might still send 'tree' if not updated, but we are asked to update frontend.
        // We will prefer 'array'.
        const remoteArray = array || tree; // Temporary fallback? Or just array?

        if (!(id instanceof RecordId) || !hash || !remoteArray) {
          return;
        }

        this.handleRemoteIncantationChange(
          id,
          hash as string,
          (remoteArray || []) as RecordVersionArray
        ).catch((err) => {
          this.logger.error({ err }, 'Error handling remote incantation change');
        });
      }
    });
  }

  private async handleRemoteIncantationChange(
    incantationId: RecordId,
    remoteHash: string,
    remoteArray: RecordVersionArray
  ) {
    // Fetch local state to get necessary params
    const existing = await this.findIncatationRecord(incantationId);
    if (!existing) {
      this.logger.warn(
        { incantationId: incantationId.toString() },
        'Received remote update for unknown local incantation'
      );
      return;
    }

    const surrealql = existing.surrealql || existing.surrealQL;
    const { params, localHash, localArray } = existing;

    await this.syncIncantation({
      incantationId,
      surrealql,
      localArray,
      localHash,
      remoteHash,
      remoteArray,
      params,
    });
  }

  private async initUpQueue() {
    await this.upQueue.loadFromDatabase();
    this.upQueue.events.subscribe(SyncQueueEventTypes.MutationEnqueued, this.syncUp.bind(this));

    /// TODO: In the future we can think about using DBSP or something smarter
    /// to update only the queries that are really affected by the change not every
    /// query that just involves this table.
    // Moved to RouterService
    // this.mutationEvents.subscribe(MutationEventTypes.MutationCreated, (event) => {
    //   const { payload } = event;
    //   for (const element of payload) {
    //     this.refreshFromLocalCache(element.record_id.table.toString());
    //   }
    // });
  }

  private async initDownQueue() {
    this.downQueue.events.subscribe(
      SyncQueueEventTypes.QueryItemEnqueued,
      this.syncDown.bind(this)
    );
  }

  private async syncUp() {
    if (this.isSyncingUp) return;
    this.isSyncingUp = true;
    try {
      while (this.upQueue.size > 0) {
        await this.upQueue.next(this.processUpEvent.bind(this));
      }
    } finally {
      this.isSyncingUp = false;
      void this.syncDown();
    }
  }

  private async syncDown() {
    if (this.isSyncingDown) return;
    if (this.upQueue.size > 0) return;

    this.isSyncingDown = true;
    try {
      while (this.downQueue.size > 0) {
        if (this.upQueue.size > 0) break;
        await this.downQueue.next(this.processDownEvent.bind(this));
      }
    } finally {
      this.isSyncingDown = false;
    }
  }

  public enqueueDownEvent(event: DownEvent) {
    this.downQueue.push(event);
  }

  private async processUpEvent(event: UpEvent) {
    switch (event.type) {
      case 'create':
        await this.remote.query(`CREATE $id CONTENT $data`, {
          id: event.record_id,
          data: event.data,
        });
        break;
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
        this.logger.error({ event }, 'processUpEvent unknown event type');
        return;
    }
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug({ event }, 'Processing down event');
    switch (event.type) {
      case 'register':
        return this.registerIncantation(event);
      case 'sync':
        const { incantationId, surrealql, params, localArray, localHash, remoteHash, remoteArray } =
          event.payload;
        return this.syncIncantation({
          incantationId,
          surrealql,
          localArray,
          localHash,
          remoteHash,
          remoteArray,
          params,
        });
      case 'heartbeat':
        return this.heartbeatIncantation(event);
      case 'cleanup':
        return this.cleanupIncantation(event);
    }
  }

  public async enqueueMutation(mutations: any[]) {
    for (const mutation of mutations) {
      this.upQueue.push(mutation);
    }
  }

  // Deprecated/Removed: effectively replaced by refreshIncantations + Router
  // public async refreshFromLocalCache(table: string) { ... }

  private async registerIncantation(event: RegisterEvent) {
    const {
      incantationId,
      surrealql,
      params,
      ttl,
      localHash: pLocalHash,
      localArray: pLocalArray,
    } = event.payload;

    const effectiveTtl = ttl || '10m';
    try {
      let existing = await this.findIncatationRecord(incantationId);
      this.logger.debug({ existing }, 'Register Incantation state');

      // Use payload values as fallback if existing record doesn't have them
      // This is critical for preventing the "empty start" loop if the incantation was just initialized
      // with known state from the stream processor or previous context.
      // NOTE: We use || and length checks because ?? doesn't work with empty strings/arrays
      // (they are not null/undefined, so ?? returns them instead of the fallback)
      const localHash = existing?.localHash || pLocalHash || '';
      const localArray = existing?.localArray?.length ? existing.localArray : (pLocalArray ?? []);

      await this.updateLocalIncantation(
        incantationId,
        {
          surrealql,
          params,
          localHash,
          localArray,
        },
        {
          updateRecord: existing ? false : true,
        }
      );

      const { hash: remoteHash, array: remoteArray } = await this.createRemoteIncantation(
        incantationId,
        surrealql,
        params,
        effectiveTtl
      );

      await this.syncIncantation({
        incantationId,
        surrealql,
        localArray,
        localHash,
        remoteHash,
        remoteArray,
        params,
      });
    } catch (e) {
      this.logger.error({ err: e }, 'registerIncantation error');
      throw e;
    }
  }

  private async createRemoteIncantation(
    incantationId: RecordId<string>,
    surrealql: string,
    params: any,
    ttl: string | Duration
  ) {
    const config = {
      id: incantationId.id,
      surrealQL: surrealql,
      params,
      ttl: typeof ttl === 'string' ? new Duration(ttl) : ttl,
      lastActiveAt: new Date(),
      clientId: this.clientId,
    };

    const { ttl: _, ...safeConfig } = config;

    // Delegate to remote function which handles DBSP registration & persistence
    const [{ hash, array }] = await this.remote.query<
      [{ hash: string; array: RecordVersionArray }]
    >('fn::incantation::register($config)', {
      config: safeConfig,
    });

    this.logger.debug(
      { incantationId: incantationId.toString(), hash, array },
      'createdRemoteIncantation'
    );
    return { hash, array };
  }

  private async syncIncantation({
    incantationId,
    surrealql,
    localArray,
    localHash,
    remoteHash,
    remoteArray,
    params,
  }: {
    incantationId: RecordId<string>;
    surrealql: string;
    localArray: RecordVersionArray;
    localHash: string;
    remoteHash: string;
    remoteArray: RecordVersionArray;
    params: Record<string, any>;
  }) {
    this.logger.debug(
      {
        incantationId: incantationId.toString(),
        localHash,
        remoteHash,
        localArray,
        remoteArray,
        params,
      },
      'syncIncantation'
    );

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) {
      return;
    }

    const arraySyncer = new ArraySyncer(localArray, remoteArray);
    let maxIter = 10;
    while (maxIter > 0) {
      const { added, updated, removed } = await this.cacheMissingRecords(
        arraySyncer,
        incantationId,
        remoteArray
      );
      if (added.length === 0 && updated.length === 0 && removed.length === 0) {
        break;
      }
      this.logger.debug({ added, updated, removed }, '[SpookySync] syncIncantation iteration');
      maxIter--;
      if (maxIter <= 0) {
        this.logger.warn(
          { incantationId: incantationId.toString() },
          'syncIncantation maxIter reached'
        );
      }
    }

    await this.updateLocalIncantation(
      incantationId,
      {
        surrealql,
        params,
        localHash: remoteHash, // After sync, local should match remote
        localArray: remoteArray, // After sync, local should match remote
        remoteHash,
        remoteArray,
      },
      {
        updateRecord: true,
      }
    );
  }

  /**
   * CRUD helper: Create a record in local cache and ingest into DBSP
   */
  private async createCacheEntry(
    record: Record<string, any>,
    incantationId: RecordId<string>,
    remoteVersion: number | undefined,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = record.id.table.toString();
    const fullId = record.id.toString();

    // 1. Cache in local DB
    await this.local.getClient().upsert(record.id).content(record);

    // 2. Ingest into DBSP
    const result = await this.streamProcessor.ingest(table, 'CREATE', fullId, record, true);

    // 3. Set version if provided
    if (remoteVersion !== undefined) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, remoteVersion);
      arraySyncer.update([[fullId, remoteVersion]] as RecordVersionArray);
    } else {
      for (const update of result) {
        if (update.query_id === incantationId.toString()) {
          arraySyncer.update(update.result_data);
        }
      }
    }
  }

  /**
   * CRUD helper: Update a record in local cache and ingest into DBSP
   */
  private async updateCacheEntry(
    record: Record<string, any>,
    incantationId: RecordId<string>,
    remoteVersion: number | undefined,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = record.id.table.toString();
    const fullId = record.id.toString();

    // 1. Cache in local DB
    await this.local.getClient().upsert(record.id).content(record);

    // 2. Ingest into DBSP (isOptimistic=false to not auto-increment version)
    const result = await this.streamProcessor.ingest(table, 'UPDATE', fullId, record, false);

    // 3. Set version if provided
    if (remoteVersion !== undefined) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, remoteVersion);
      arraySyncer.update([[fullId, remoteVersion]] as RecordVersionArray);
    } else {
      for (const update of result) {
        if (update.query_id === incantationId.toString()) {
          arraySyncer.update(update.result_data);
        }
      }
    }
  }

  /**
   * CRUD helper: Delete a record from local cache and ingest deletion into DBSP
   */
  private async deleteCacheEntry(
    recordId: RecordId,
    incantationId: RecordId<string>,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = recordId.table.toString();
    const fullId = recordId.toString();

    // 1. Delete from local DB
    await this.local.query('DELETE $id', { id: recordId });

    // 2. Ingest deletion into DBSP
    const result = await this.streamProcessor.ingest(table, 'DELETE', fullId, {}, false);

    for (const update of result) {
      if (update.query_id === incantationId.toString()) {
        arraySyncer.update(update.result_data);
      }
    }
  }

  /**
   * Sync missing/updated/removed records between local and remote.
   * Handles CREATE, UPDATE, and DELETE operations.
   */
  private async cacheMissingRecords(
    arraySyncer: ArraySyncer,
    incantationId: RecordId<string>,
    remoteArray: RecordVersionArray
  ): Promise<RecordVersionDiff> {
    const diff = arraySyncer.nextSet();
    const { added, updated, removed } = diff;
    const idsToFetch = [...added, ...updated];

    this.logger.debug({ added, updated, removed, idsToFetch }, 'cacheMissingRecords diff');

    // Build remote version map for quick lookup
    const remoteVersionMap = new Map<string, number>();
    for (const [id, ver] of remoteArray) {
      remoteVersionMap.set(id.toString(), ver);
    }

    // Handle removed records: verify they don't exist remotely before deleting locally
    // This prevents deleting records that might still exist due to stale remoteArray
    if (removed.length > 0) {
      this.logger.debug({ removed: removed.map((r) => r.toString()) }, 'Checking removed records');

      const [existingRemote] = await this.remote.query<[{ id: RecordId }[]]>(
        'SELECT id FROM $ids',
        { ids: removed }
      );
      const existingRemoteIds = new Set(existingRemote.map((r) => r.id.toString()));

      for (const recordId of removed) {
        if (!existingRemoteIds.has(recordId.toString())) {
          this.logger.debug({ recordId: recordId.toString() }, 'Deleting confirmed removed record');
          await this.deleteCacheEntry(recordId, incantationId, arraySyncer);
        }
      }
    }

    // Fetch added/updated records from remote
    if (idsToFetch.length === 0) {
      return { added: [], updated: [], removed };
    }

    const [remoteResults] = await this.remote.query<[Record<string, any>[]]>('SELECT * FROM $ids', {
      ids: idsToFetch,
    });

    // Flatten and prepare results
    const flatResults = this.flattenResults(remoteResults);

    // Handle added records
    for (const record of flatResults) {
      const fullId = record.id.toString();
      const isAdded = added.some((id) => id.toString() === fullId);
      const isUpdated = updated.some((id) => id.toString() === fullId);
      const remoteVer = remoteVersionMap.get(fullId);

      if (isAdded) {
        await this.createCacheEntry(record, incantationId, remoteVer, arraySyncer);
      } else if (isUpdated) {
        await this.updateCacheEntry(record, incantationId, remoteVer, arraySyncer);
      }
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: flatResults,
    });

    return { added, updated, removed };
  }

  private async updateLocalIncantation(
    incantationId: RecordId<string>,
    {
      surrealql,
      params,
      localHash,
      localArray,
      remoteHash,
      remoteArray,
    }: {
      surrealql: string;
      params?: Record<string, any>;
      localHash?: string;
      localArray?: RecordVersionArray;
      remoteHash?: string;
      remoteArray?: RecordVersionArray;
    },
    {
      updateRecord = true,
    }: {
      updateRecord?: boolean;
    }
  ) {
    if (updateRecord) {
      const content: any = {};
      if (localHash !== undefined) content.localHash = localHash;
      if (localArray !== undefined) content.localArray = localArray;
      if (remoteHash !== undefined) content.remoteHash = remoteHash;
      if (remoteArray !== undefined) content.remoteArray = remoteArray;

      await this.updateIncantationRecord(incantationId, content);
    }

    try {
      this.logger.debug(
        {
          incantationId: incantationId.toString(),
          surrealql,
          params,
        },
        'updateLocalIncantation Loading cached results start'
      );

      const [cachedResults] = await this.local.query<[Record<string, any>[]]>(surrealql, params);

      // Verify Orphans if we have a remote tree to check against
      // if (remoteTree) {
      //   void this.verifyAndPurgeOrphans(cachedResults, remoteTree);
      // }

      this.logger.debug(
        {
          incantationId: incantationId.toString(),
          recordCount: cachedResults?.length,
        },
        'updateLocalIncantation Loading cached results done'
      );

      this.events.emit(SyncEventTypes.IncantationUpdated, {
        incantationId,
        localHash,
        localArray,
        remoteHash,
        remoteArray,
        records: cachedResults || [],
      });
    } catch (e) {
      this.logger.error(
        { err: e },
        'updateLocalIncantation failed to query local db or emit event'
      );
    }
  }

  private async findIncatationRecord(incantationId: RecordId<string>) {
    try {
      const [cachedResults] = await this.local.query<[Record<string, any>]>(
        'SELECT * FROM ONLY $id',
        {
          id: incantationId,
        }
      );
      return cachedResults;
    } catch (e) {
      return null;
    }
  }

  private async updateIncantationRecord(
    incantationId: RecordId<string>,
    content: Record<string, any>
  ) {
    try {
      this.logger.debug(
        { incantationId: incantationId.toString(), content },
        'Updating local incantation'
      );
      await this.local.query(`UPDATE $id MERGE $content`, {
        id: incantationId,
        content,
      });
    } catch (e) {
      this.logger.error({ err: e }, 'Failed to update local incantation record');
      throw e;
    }
  }

  /**
   * Recursively flattens a list of records, extracting nested objects that look like records (have an 'id')
   * into the top-level list, and replacing them with their ID in the parent.
   *
   * schema-aware: Only flattens fields that are defined as relationships in the schema for the specific table.
   */
  // TODO: Move this to utils
  private flattenResults(
    results: Record<string, any>[],
    visited: Set<string> = new Set(),
    flattened: Record<string, any>[] = []
  ): Record<string, any>[] {
    for (const record of results) {
      if (!record) continue;

      // 1. Identify the Record
      let recordIdStr: string | undefined;
      let tableName: string | undefined;

      if (record.id && record.id instanceof RecordId) {
        recordIdStr = record.id.toString();
        tableName = record.id.table.name;
      }

      // 2. Cycle Detection / Deduplication
      if (recordIdStr) {
        if (visited.has(recordIdStr)) continue;
        visited.add(recordIdStr);
      }

      // 3. Create a shallow copy to modify fields without mutating original
      const processedRecord: Record<string, any> = { ...record };

      // 4. Handle Relationships recursively
      if (tableName && this.relationshipMap.has(tableName)) {
        const validFields = this.relationshipMap.get(tableName)!;

        for (const key of validFields) {
          if (!(key in processedRecord)) continue;

          const value = processedRecord[key];

          if (value && typeof value === 'object') {
            if (Array.isArray(value)) {
              processedRecord[key] = this.flattenResults(value, visited, flattened);
            } else if (value.id && value.id instanceof RecordId) {
              this.flattenResults([value], visited, flattened);
              processedRecord[key] = value.id;
            }
          }
        }
      }
      flattened.push(processedRecord);
    }

    return flattened;
  }

  private async heartbeatIncantation(event: HeartbeatEvent) {
    await this.remote.query('fn::incantation::heartbeat($id)', {
      id: event.payload.incantationId,
    });
  }

  private async cleanupIncantation(event: CleanupEvent) {
    await this.remote.query(`DELETE $id`, {
      id: event.payload.incantationId,
    });
  }
}
