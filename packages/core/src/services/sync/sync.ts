import { LocalDatabaseService, RemoteDatabaseService } from "../database/index.js";
import { MutationEventSystem } from "../mutation/index.js";
import { Incantation as IncantationData } from "../../types.js";
import { QueryEventSystem, QueryEventTypes } from "../query/events.js";
import { SyncQueueEventTypes } from "./events.js";
import { CleanupEvent, DownEvent, DownQueue, HeartbeatEvent, RegisterEvent, SyncEvent, UpEvent, UpQueue } from "./queue/index.js";
import { RecordId } from "surrealdb";

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
    if (this.isInit) throw new Error("SpookySync is already initialized");
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
    this.downQueue.events.subscribeMany([
      SyncQueueEventTypes.IncantationRegistrationEnqueued,
      SyncQueueEventTypes.IncantationSyncEnqueued,
      SyncQueueEventTypes.IncantationCleanupEnqueued,
    ], this.syncDown.bind(this));
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
    switch  (event.type) {
        case "create":
            await this.remote.query(`CREATE $id CONTENT $data`, {
                id: event.record_id,
                data: event.data,
            });
            break;
        case "update":
            await this.remote.query(`UPDATE $id MERGE $data`, {
                id: event.record_id,
                data: event.data,
            });
            break;
        case "delete":
            await this.remote.query(`DELETE $id`, {
                id: event.record_id,
            });
            break;
        }
  }

  private async processDownEvent(event: DownEvent) {
    switch (event.type) {
        case "register":
            return this.registerIncantation(event);
        case "sync":
            return this.syncIncantation(event);
        case "heartbeat":
            return this.heartbeatIncantation(event);
        case "cleanup":
            return this.cleanupIncantation(event);
        }
  }

  private async registerIncantation(event: RegisterEvent) {
    const { incantationId, surrealql, ttl } = event.payload;
    await this.remote.getClient().upsert(incantationId).content({
      surrealql,
      ttl,
    });
  }

  private async syncIncantation(event: SyncEvent) {
    const { incantationId, surrealql, localTree, localHash, remoteHash, remoteTree } = event.payload;

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) return;

    if (!localHash || !localTree) {
      const [remoteResults] = await this.remote.getClient().query(surrealql).collect<[Record<string, any>[]]>();
      await this.updateLocalIncantation(incantationId, {
        hash: remoteHash,
        tree: remoteTree,
        records: remoteResults,
      });
      return;
    }

    // TODO: tree comparsion and single record updates
  }

  private async updateLocalIncantation(incantationId: RecordId<string>, {
    hash,
    tree,
    records,
  }: {
    hash: string;
    tree: any;
    records: Record<string, any>[];
  }) {
    await this.updateIncantationRecord(incantationId, {
      hash,
      tree,
    });

    this.queryEvents.emit(QueryEventTypes.IncantationIncomingRemoteUpdate, {
      incantationId,
      remoteHash: hash,
      remoteTree: tree,
      records,
    });

    await this.cacheResults(records);
  }

  private async updateIncantationRecord(incantationId: RecordId<string>, {
    hash,
    tree,
  }: {
    hash: string;
    tree: any;
  }) {
    await this.local.getClient().update(incantationId).content({
      hash,
      tree,
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
    await this.remote.getClient().query("fn::incantation::heartbeat($id)", {
      id: event.payload.incantationId,
    });
  }

  private async cleanupIncantation(event: CleanupEvent) {
    await this.remote.getClient().delete(event.payload.incantationId);
  }
}