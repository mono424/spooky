import { LocalDatabaseService, RemoteDatabaseService } from '../database/index.js';
import { MutationEventTypes } from '../mutation/index.js';
import { IdTreeDiff } from '../../types.js';
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
import { diffIdTree, flattenIdTree } from './utils.js';
import { parseRecordIdString, decodeFromSpooky } from '../utils/index.js';
import { TableModel } from '@spooky/query-builder';
import { SchemaStructure } from '@spooky/query-builder';
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
        const { id, hash, tree } = message.value;
        if (!(id instanceof RecordId) || !hash || !tree) {
          return;
        }

        this.handleRemoteIncantationChange(id, hash as string, tree).catch((err) => {
          this.logger.error({ err }, 'Error handling remote incantation change');
        });
      }
    });
  }

  private async handleRemoteIncantationChange(
    incantationId: RecordId,
    remoteHash: string,
    remoteTree: any
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

    const { surrealql, params, hash: localHash, tree: localTree } = existing;

    await this.syncIncantation({
      incantationId,
      surrealql,
      localTree,
      localHash,
      remoteHash,
      remoteTree,
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
        await this.registerIncantation(event);
        return;
      case 'sync':
        const { incantationId, surrealql, params, localTree, localHash, remoteHash, remoteTree } =
          event.payload;
        return this.syncIncantation({
          incantationId,
          surrealql,
          localTree,
          localHash,
          remoteHash,
          remoteTree,
          params,
        });
      case 'heartbeat':
        return this.heartbeatIncantation(event);
      case 'cleanup':
        return this.cleanupIncantation(event);
    }
  }

  public async refreshIncantations(
    queries: { id: RecordId; surrealql: string; params?: Record<string, any> }[]
  ) {
    this.logger.debug({ count: queries.length }, 'refreshIncantations');
    for (const query of queries) {
      await this.updateLocalIncantation(
        query.id,
        {
          surrealql: query.surrealql,
          params: query.params,
          hash: '',
          tree: null,
        },
        {
          updateRecord: false,
        }
      );
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
    const { incantationId, surrealql, params, ttl } = event.payload;
    const effectiveTtl = ttl || '10m';
    try {
      let existing = await this.findIncatationRecord(incantationId);
      this.logger.debug({ existing }, 'Register Incantation state');

      const localHash = existing?.hash ?? '';
      const localTree = existing?.tree ?? null;

      await this.updateLocalIncantation(
        incantationId,
        {
          surrealql,
          params,
          hash: localHash,
          tree: localTree,
        },
        {
          updateRecord: existing ? false : true,
        }
      );

      const { hash: remoteHash, tree: remoteTree } = await this.createRemoteIncantation(
        incantationId,
        surrealql,
        params,
        effectiveTtl
      );

      await this.syncIncantation({
        incantationId,
        surrealql,
        localTree,
        localHash,
        remoteHash,
        remoteTree,
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
    const [{ hash, tree }] = await this.remote.query<[{ hash: string; tree: any }]>(
      'fn::incantation::register($config)',
      {
        config: safeConfig,
      }
    );

    this.logger.debug(
      { incantationId: incantationId.toString(), hash, tree },
      'createdRemoteIncantation'
    );
    return { hash, tree };
  }

  private async syncIncantation({
    incantationId,
    surrealql,
    localTree,
    localHash,
    remoteHash,
    remoteTree,
    params,
  }: {
    incantationId: RecordId<string>;
    surrealql: string;
    localTree: any;
    localHash: string;
    remoteHash: string;
    remoteTree: any;
    params: Record<string, any>;
  }) {
    this.logger.debug(
      {
        incantationId: incantationId.toString(),
        localHash,
        remoteHash,
        localTree,
        remoteTree,
        params,
      },
      'syncIncantation'
    );

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) {
      return;
    }

    await this.cacheMissingRecords(localTree, remoteTree, surrealql);

    await this.updateLocalIncantation(
      incantationId,
      {
        surrealql,
        params,
        hash: remoteHash,
        tree: remoteTree,
      },
      {
        updateRecord: true,
      }
    );
  }

  private async cacheMissingRecords(
    localTree: any,
    remoteTree: any,
    surrealql: string
  ): Promise<IdTreeDiff> {
    if (!localTree) {
      const [remoteResults] = await this.remote.query<[Record<string, any>[]]>(surrealql);
      // TODO: flatten the records array, to not have nested dependencies but a flat list of records
      // for this it should use the schema to find relationships
      await this.cacheResults(remoteResults);
      return { added: remoteResults.map((r) => r.id), updated: [], removed: [] };
    }

    const diff = diffIdTree(localTree, remoteTree);
    const { added, updated } = diff;
    const idsToFetch = [...added, ...updated];

    this.logger.debug({ added, updated, idsToFetch }, 'cacheMissingRecords diff');

    if (idsToFetch.length === 0) {
      return { added: [], updated: [], removed: [] };
    }

    const [remoteResults] = await this.remote.query<[Record<string, any>[]]>('SELECT * FROM $ids', {
      ids: idsToFetch,
    });

    await this.cacheResults(remoteResults);
    return { added: remoteResults.map((r) => r.id), updated: [], removed: [] };
  }

  private async updateLocalIncantation(
    incantationId: RecordId<string>,
    {
      surrealql,
      params,
      hash,
      tree,
    }: {
      surrealql: string;
      params?: Record<string, any>;
      hash: string;
      tree: any;
    },
    {
      updateRecord = true,
    }: {
      updateRecord?: boolean;
    }
  ) {
    if (updateRecord) {
      await this.updateIncantationRecord(incantationId, {
        hash,
        tree,
      });
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
      if (tree) {
        void this.verifyAndPurgeOrphans(cachedResults, tree);
      }

      this.logger.debug(
        {
          incantationId: incantationId.toString(),
          recordCount: cachedResults?.length,
        },
        'updateLocalIncantation Loading cached results done'
      );

      this.events.emit(SyncEventTypes.IncantationUpdated, {
        incantationId,
        remoteHash: hash,
        remoteTree: tree,
        records: cachedResults,
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

  private async verifyAndPurgeOrphans(cachedResults: any[], remoteTree: any) {
    if (!cachedResults || cachedResults.length === 0 || !remoteTree) return;

    const remoteIds = new Set(flattenIdTree(remoteTree).map((node: any) => node.id));
    const orphans: any[] = [];

    for (const r of cachedResults) {
      // We need to decode? cachedResults are from local DB, they are structured.
      // BUT they might need to be flattened or handled if the query returns nested stuff?
      // For now assume top level IDs.
      // QueryManager did decodeFromSpooky. Let's assume local DB returns raw results that match
      // what RecordId.toString() expects?
      // Wait, QueryManager used `r.id`.
      const id = r.id;
      const idStr = id instanceof RecordId ? id.toString() : id;

      if (idStr && !remoteIds.has(idStr)) {
        orphans.push(r);
      }
    }

    if (orphans.length === 0) return;

    const idsToCheck = orphans
      .map((r) => r.id)
      .filter((id) => !!id)
      .map((id) => (id instanceof RecordId ? id : parseRecordIdString(id.toString())));

    if (idsToCheck.length === 0) return;

    this.logger.debug({ count: idsToCheck.length }, 'Verifying orphaned records against remote');

    try {
      const [existing] = await this.remote.query<[{ id: RecordId }[]]>('SELECT id FROM $ids', {
        ids: idsToCheck,
      });

      const existingIdsSet = new Set(existing.map((r) => r.id.toString()));
      const toDelete = idsToCheck.filter((id) => !existingIdsSet.has(id.toString()));

      if (toDelete.length > 0) {
        this.logger.info(
          { count: toDelete.length, ids: toDelete.map((id) => id.toString()) },
          'Purging confirmed orphaned records'
        );
        await this.local.query('DELETE $ids', { ids: toDelete });
      } else {
        this.logger.debug(
          { count: idsToCheck.length },
          'All orphaned records still exist remotely (ghost records checking)'
        );
      }
    } catch (err) {
      this.logger.error({ err }, 'Failed to verify/purge orphans');
    }
  }

  private async updateIncantationRecord(
    incantationId: RecordId<string>,
    {
      hash,
      tree,
    }: {
      hash: string;
      tree: any;
    }
  ) {
    try {
      this.logger.debug(
        { incantationId: incantationId.toString(), hash, tree },
        'Updating local incantation'
      );
      await this.local.query(`UPDATE $id MERGE $content`, {
        id: incantationId,
        content: { hash, tree },
      });
    } catch (e) {
      this.logger.error({ err: e }, 'Failed to update local incantation record');
      throw e;
    }
  }

  private async cacheResults(results: Record<string, any>[]) {
    if (!results || results.length === 0) return;
    this.logger.trace({ results }, 'cacheResults raw');
    const flatResults = this.flattenResults(results);
    this.logger.trace({ flatResults }, 'cacheResults flattened');

    for (const record of flatResults) {
      if (record.id) {
        await this.local.getClient().upsert(record.id).content(record);
      }
    }
  }

  /**
   * Recursively flattens a list of records, extracting nested objects that look like records (have an 'id')
   * into the top-level list, and replacing them with their ID in the parent.
   *
   * schema-aware: Only flattens fields that are defined as relationships in the schema for the specific table.
   */
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
