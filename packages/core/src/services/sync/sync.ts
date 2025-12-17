import { LocalDatabaseService, RemoteDatabaseService } from "../database/index.js";
import { MutationEventSystem, MutationEventTypes } from "../mutation/index.js";
import { SyncQueueEventTypes } from "./events.js";
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
    this.upQueue.events.subscribe(SyncQueueEventTypes.MutationEnqueued, this.sync.bind(this));
    this.upQueue.listenForMutations(this.mutationEvents);
    void this.sync();
  }

  private async sync() {
    if (this.isSyncing) return;
    this.isSyncing = true;
    try {
        while (this.upQueue.size > 0) {
            await this.upQueue.next(this.processUpEvent.bind(this));
        }
    } finally {
        this.isSyncing = false;
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
}