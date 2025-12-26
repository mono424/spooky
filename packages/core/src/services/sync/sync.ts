import { LocalDatabaseService, RemoteDatabaseService } from "../database/index.js";
import { MutationEventSystem } from "../mutation/index.js";
import { QueryEventSystem } from "../query/events.js";
import { SyncQueueEventTypes } from "./events.js";
import { CleanupEvent, DownEvent, DownQueue, HeartbeatEvent, RegisterEvent, SyncEvent, UpEvent, UpQueue } from "./queue/index.js";

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
    /// TODO get local tree, compare with remote tree, find diff, request diff
  }

  private async heartbeatIncantation(event: HeartbeatEvent) {
    await this.remote.getClient().update(event.payload.incantationId).patch({
      lastActiveAt: new Date(),
    });
  }

  private async cleanupIncantation(event: CleanupEvent) {
    await this.remote.getClient().delete(event.payload.incantationId);
  }
}