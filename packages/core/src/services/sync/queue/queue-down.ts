import { LocalDatabaseService } from '../../database/index.js';
import {
  createSyncQueueEventSystem,
  SyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events.js';
import { QueryEventTypeMap } from '../../query/events.js';
import { EventPayload } from '../../../events/index.js';

export type RegisterEvent = {
  type: 'register';
  payload: EventPayload<QueryEventTypeMap, 'QUERY_INCANTATION_INITIALIZED'>;
};

export type SyncEvent = {
  type: 'sync';
  payload: EventPayload<QueryEventTypeMap, 'QUERY_INCANTATION_REMOTE_HASH_UPDATE'>;
};

export type HeartbeatEvent = {
  type: 'heartbeat';
  payload: EventPayload<QueryEventTypeMap, 'QUERY_INCANTATION_TTL_HEARTBEAT'>;
};

export type CleanupEvent = {
  type: 'cleanup';
  payload: EventPayload<QueryEventTypeMap, 'QUERY_INCANTATION_CLEANUP'>;
};

export type DownEvent = RegisterEvent | SyncEvent | HeartbeatEvent | CleanupEvent;

export class DownQueue {
  private queue: DownEvent[] = [];
  private _events: SyncQueueEventSystem;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(private local: LocalDatabaseService) {
    this._events = createSyncQueueEventSystem();
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
        console.error('Failed to process query', event, error);
        this.queue.unshift(event);
        throw error;
      }
    }
  }
}
