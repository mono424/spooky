import { LocalDatabaseService, RemoteDatabaseService } from "../database/index.js";
import { MutationEventSystem, MutationEventTypes } from "../mutation/index.js";
import { UpEvent, UpQueue } from "./queue.js";

export class SpookySync {
  private upQueue: UpQueue;
  private isInit: boolean = false;
  private isSyncing: boolean = false;

  constructor(private local: LocalDatabaseService, private remote: RemoteDatabaseService, private mutationEvents: MutationEventSystem) {
    this.upQueue = new UpQueue(this.local);
  }

  public async init() {
    if (this.isInit) throw new Error("SpookySync is already initialized");
    this.isInit = true;
    await this.upQueue.loadFromDatabase();
    this.upQueue.registerEnqueueListener(this.triggerSync.bind(this));
    this.upQueue.listenForMutations(this.mutationEvents);
    // Trigger initial sync for any loaded items
    void this.triggerSync();
  }

  private async triggerSync() {
    if (this.isSyncing) return;
    this.isSyncing = true;
    try {
        while (this.upQueue.size > 0) {
            await this.syncNext();
        }
    } finally {
        this.isSyncing = false;
    }
  }

  private async syncNext() {
    await this.upQueue.next(this.processUpEvent.bind(this));
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
}