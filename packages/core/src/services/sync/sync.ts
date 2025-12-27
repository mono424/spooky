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
import { RecordId } from 'surrealdb';
import { diffIdTree } from './utils.js';

export class SpookySync {
  private upQueue: UpQueue;
  private downQueue: DownQueue;
  private isInit: boolean = false;
  private isSyncingUp: boolean = false;
  private isSyncingDown: boolean = false;

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private mutationEvents: MutationEventSystem,
    private queryEvents: QueryEventSystem
  ) {
    this.upQueue = new UpQueue(this.local);
    this.downQueue = new DownQueue(this.local);
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
    const { incantationId, surrealql, ttl } = event.payload;
    await this.updateLocalIncantation(incantationId, {
      surrealql,
      hash: '',
      tree: null,
    });
    await this.remote.query(`UPSERT $id CONTENT $content`, {
      id: incantationId,
      content: { surrealql, ttl },
    });
  }

  private async syncIncantation(event: SyncEvent) {
    const { incantationId, surrealql, localTree, localHash, remoteHash, remoteTree } =
      event.payload;

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) return;

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
      hash,
      tree,
    }: {
      surrealql: string;
      hash: string;
      tree: any;
    }
  ) {
    await this.updateIncantationRecord(incantationId, {
      hash,
      tree,
    });

    const cachedResults = await this.local
      .getClient()
      .query(surrealql)
      .collect<[Record<string, any>[]]>();

    this.queryEvents.emit(QueryEventTypes.IncantationIncomingRemoteUpdate, {
      incantationId,
      remoteHash: hash,
      remoteTree: tree,
      records: cachedResults,
    });
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

  // TODO: support joined records
  private async cacheResults(results: Record<string, any>[]) {
    const tx = await this.local.getClient().beginTransaction();
    for (const record of results) {
      if (record.id) {
        await tx.upsert(record.id).content(record);
      }
    }
    await tx.commit();
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
