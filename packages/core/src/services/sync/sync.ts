import { LocalDatabaseService, RemoteDatabaseService } from '../database/index.js';
import { MutationEventSystem } from '../mutation/index.js';
import { IdTreeDiff, Incantation as IncantationData } from '../../types.js';
import { QueryEventSystem, QueryEventTypes } from '../query/events.js';
import { SyncQueueEventTypes } from './events.js';
import {
  CleanupEvent,
  DownEvent,
  DownQueue,
  HeartbeatEvent,
  RegisterEvent,
  SyncEvent,
  UpEvent,
  UpQueue,
} from './queue/index.js';
import { RecordId, Duration } from 'surrealdb';
import { diffIdTree } from './utils.js';
import { SchemaStructure } from '@spooky/query-builder';

export class SpookySync<S extends SchemaStructure> {
  private upQueue: UpQueue;
  private downQueue: DownQueue;
  private isInit: boolean = false;
  private isSyncingUp: boolean = false;
  private isSyncingDown: boolean = false;
  private relationshipMap: Map<string, Set<string>> = new Map();

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }

  constructor(
    private schema: S,
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private mutationEvents: MutationEventSystem,
    private queryEvents: QueryEventSystem,
    private clientId: string
  ) {
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
    console.log('syncing down');
    if (this.isInit) throw new Error('SpookySync is already initialized');
    this.isInit = true;
    await this.initUpQueue();
    await this.initDownQueue();
    void this.syncUp();
    void this.syncDown();
  }

  private async initUpQueue() {
    await this.upQueue.loadFromDatabase();
    this.upQueue.events.subscribe(SyncQueueEventTypes.MutationEnqueued, this.syncUp.bind(this));
    this.upQueue.listenForMutations(this.mutationEvents);
  }

  private async initDownQueue() {
    this.downQueue.events.subscribeMany(
      [
        SyncQueueEventTypes.IncantationRegistrationEnqueued,
        SyncQueueEventTypes.IncantationSyncEnqueued,
        SyncQueueEventTypes.IncantationCleanupEnqueued,
      ],
      this.syncDown.bind(this)
    );
    this.downQueue.listenForQueries(this.queryEvents);
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
    }
  }

  private async processDownEvent(event: DownEvent) {
    console.log('down event', event);
    switch (event.type) {
      case 'register':
        return this.registerIncantation(event);
      case 'sync':
        return this.syncIncantation(event);
      case 'heartbeat':
        return this.heartbeatIncantation(event);
      case 'cleanup':
        return this.cleanupIncantation(event);
    }
  }

  private async registerIncantation(event: RegisterEvent) {
    const { incantationId, surrealql, params, ttl } = event.payload;
    const effectiveTtl = ttl || '10m';
    try {
      await this.updateLocalIncantation(incantationId, {
        surrealql,
        params,
        hash: '',
        tree: null,
      });
      await this.remote.query(`UPSERT $id CONTENT $content`, {
        id: incantationId,
        content: {
          SurrealQL: surrealql,
          Params: params,
          TTL: new Duration(effectiveTtl),
          Id: incantationId.id.toString(),
          LastActiveAt: new Date(),
          ClientId: this.clientId,
        },
      });
    } catch (e) {
      console.error('[SpookySync] registerIncantation error', e);
      throw e;
    }
  }

  private async syncIncantation(event: SyncEvent) {
    const { incantationId, surrealql, localTree, localHash, remoteHash, remoteTree } =
      event.payload;

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) {
      return;
    }

    await this.cacheMissingRecords(localTree, remoteTree, surrealql);

    await this.updateLocalIncantation(incantationId, {
      surrealql,
      hash: remoteHash,
      tree: remoteTree,
    });
  }

  private async cacheMissingRecords(
    localTree: any,
    remoteTree: any,
    surrealql: string
  ): Promise<IdTreeDiff> {
    if (!localTree) {
      const [remoteResults] = await this.remote
        .getClient()
        .query(surrealql)
        .collect<[Record<string, any>[]]>();
      // TODO: flatten the records array, to not have nested dependencies but a flat list of records
      // for this it should use the schema to find relationships
      await this.cacheResults(remoteResults);
      return { added: remoteResults.map((r) => r.id), updated: [], removed: [] };
    }

    const diff = diffIdTree(localTree, remoteTree);
    const { added, updated } = diff;
    const idsToFetch = [...added, ...updated];

    if (idsToFetch.length === 0) {
      return { added: [], updated: [], removed: [] };
    }

    const [remoteResults] = await this.remote
      .getClient()
      .query('SELECT * FROM $ids', { ids: idsToFetch })
      .collect<[Record<string, any>[]]>();

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
    }
  ) {
    await this.updateIncantationRecord(incantationId, {
      hash,
      tree,
    });

    try {
      const [cachedResults] = await this.local
        .getClient()
        .query(surrealql, params)
        .collect<[Record<string, any>[]]>();

      this.queryEvents.emit(QueryEventTypes.IncantationIncomingRemoteUpdate, {
        incantationId,
        remoteHash: hash,
        remoteTree: tree,
        records: cachedResults,
      });
    } catch (e) {
      console.error('[SpookySync] failed to query local db or emit event', e);
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
    await this.local.query(`UPDATE $id MERGE $content`, {
      id: incantationId,
      content: { hash, tree },
    });
  }

  private async cacheResults(results: Record<string, any>[]) {
    if (!results || results.length === 0) return;
    console.log(results);
    const flatResults = this.flattenResults(results);
    console.log('Flattend', {
      results,
      flatResults,
    });
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
    await this.remote.getClient().query('fn::incantation::heartbeat($id)', {
      id: event.payload.incantationId,
    });
  }

  private async cleanupIncantation(event: CleanupEvent) {
    await this.remote.query(`DELETE $id`, {
      id: event.payload.incantationId,
    });
  }
}
