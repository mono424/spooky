import { LocalDatabaseService } from '../../../services/database/index.js';
import {
  createSyncQueueEventSystem,
  SyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events/index.js';
import { Logger } from '../../../services/logger/index.js';

export type RegisterEvent = {
  type: 'register';
  payload: {
    hash: string;
  };
};

export type SyncEvent = {
  type: 'sync';
  payload: {
    hash: string;
  };
};

export type HeartbeatEvent = {
  type: 'heartbeat';
  payload: {
    hash: string;
  };
};

export type CleanupEvent = {
  type: 'cleanup';
  payload: {
    hash: string;
  };
};

export type DownEvent = RegisterEvent | SyncEvent | HeartbeatEvent | CleanupEvent;

export class DownQueue {
  private queue: DownEvent[] = [];
  private _events: SyncQueueEventSystem;
  private logger: Logger;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(
    private local: LocalDatabaseService,
    logger: Logger
  ) {
    this._events = createSyncQueueEventSystem();
    this.logger = logger.child({ service: 'DownQueue' });
  }

  get size(): number {
    return this.queue.length;
  }

  push(event: DownEvent) {
    this.queue.push(event);
    this.emitPushEvent();
  }

  private emitPushEvent() {
    this._events.addEvent({
      type: SyncQueueEventTypes.QueryItemEnqueued,
      payload: {
        queueSize: this.queue.length,
      },
    });
  }

  async next(fn: (event: DownEvent) => Promise<void>): Promise<void> {
    const event = this.queue.shift();
    if (event) {
      try {
        await fn(event);
      } catch (error) {
        this.logger.error(
          { error, event, Category: 'spooky-client::DownQueue::next' },
          'Failed to process query'
        );
        this.queue.unshift(event);
        throw error;
      }
    }
  }
}
