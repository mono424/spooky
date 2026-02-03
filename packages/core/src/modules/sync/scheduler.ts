import { Logger } from '../../services/logger/index.js';
import { UpQueue, DownQueue, DownEvent, UpEvent } from './queue/index.js';
import { SyncQueueEventTypes } from './events/index.js';

/**
 * SyncScheduler manages when to sync: queue management and orchestration.
 * Decides the order and timing of sync operations.
 */
export class SyncScheduler {
  private isSyncingUp: boolean = false;
  private isSyncingDown: boolean = false;

  constructor(
    private upQueue: UpQueue,
    private downQueue: DownQueue,
    private onProcessUp: (event: UpEvent) => Promise<void>,
    private onProcessDown: (event: DownEvent) => Promise<void>,
    private logger: Logger
  ) {}

  async init() {
    await this.upQueue.loadFromDatabase();
    this.upQueue.events.subscribe(SyncQueueEventTypes.MutationEnqueued, this.syncUp.bind(this));
    this.downQueue.events.subscribe(
      SyncQueueEventTypes.QueryItemEnqueued,
      this.syncDown.bind(this)
    );
  }

  /**
   * Add mutations to the upload queue
   */
  enqueueMutation(mutations: UpEvent[]) {
    for (const mutation of mutations) {
      this.upQueue.push(mutation);
    }
  }

  /**
   * Add query events to the download queue
   */
  enqueueDownEvent(event: DownEvent) {
    this.downQueue.push(event);
  }

  /**
   * Process upload queue
   */
  async syncUp() {
    if (this.isSyncingUp) return;
    this.isSyncingUp = true;
    try {
      while (this.upQueue.size > 0) {
        await this.upQueue.next(this.onProcessUp);
      }
    } finally {
      this.isSyncingUp = false;
      void this.syncDown();
    }
  }

  /**
   * Process download queue
   */
  async syncDown() {
    if (this.isSyncingDown) return;
    if (this.upQueue.size > 0) return;

    this.isSyncingDown = true;
    try {
      while (this.downQueue.size > 0) {
        if (this.upQueue.size > 0) break;
        await this.downQueue.next(this.onProcessDown);
      }
    } finally {
      this.isSyncingDown = false;
    }
  }

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }
}
