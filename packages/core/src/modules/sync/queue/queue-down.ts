import type { LocalStore } from '../../../services/database/index';
import type {
  SyncQueueEventSystem} from '../events/index';
import {
  createSyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events/index';
import type { Logger } from '../../../services/logger/index';

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

/**
 * How many times a failing event keeps its place at the head before it is
 * rotated to the back. A transient failure (the SSP still bootstrapping) clears
 * well inside this, so ordering is preserved for the common case; a permanently
 * rejected event (a permission the SSP refuses to lower) stops holding every
 * other query hostage behind it.
 */
const MAX_HEAD_RETRIES = 3;

export class DownQueue {
  private queue: DownEvent[] = [];
  private _events: SyncQueueEventSystem;
  private logger: Logger;
  /** Consecutive failures per queued event; cleared when it finally succeeds. */
  private failures = new WeakMap<DownEvent, number>();

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(
    private local: LocalStore,
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

  /** Drop all queued events (bucket switch: old-bucket register/sync work is
   *  re-derived after the switch; replaying it would target dead query rows). */
  clear(): void {
    this.queue = [];
  }

  async next(fn: (event: DownEvent) => Promise<void>): Promise<void> {
    const event = this.queue.shift();
    if (!event) return;
    const error = await this.run(event, fn);
    if (error !== undefined) throw error;
  }

  /**
   * The next event whose hash is NOT already being processed, or `undefined`
   * when every remaining event is blocked on a busy hash.
   *
   * Ordering only ever mattered PER HASH — a `cleanup` must not overtake the
   * `register` for the same query — but the queue enforced it globally, so one
   * registration RPC at a time was the ceiling for the whole client. An event
   * for a busy hash keeps its place here (it is skipped, not reordered), so
   * per-hash ordering is preserved exactly while independent hashes proceed
   * concurrently.
   */
  takeNext(busy: ReadonlySet<string>): DownEvent | undefined {
    for (let i = 0; i < this.queue.length; i++) {
      const event = this.queue[i]!;
      if (busy.has(event.payload.hash)) continue;
      this.queue.splice(i, 1);
      return event;
    }
    return undefined;
  }

  /**
   * Process one event, applying the re-head / rotate failure policy.
   *
   * NEVER rejects: it RETURNS the error instead (`undefined` on success). A
   * concurrent drain has other work in flight when one event fails, and a
   * rejection would either take that work down with it or have to be caught at
   * every call site. `next` reinstates the throwing contract for the serial
   * callers that still want it.
   */
  async run(
    event: DownEvent,
    fn: (event: DownEvent) => Promise<void>
  ): Promise<unknown | undefined> {
    try {
      await fn(event);
      this.failures.delete(event);
      return undefined;
    } catch (error) {
      const attempts = (this.failures.get(event) ?? 0) + 1;
      this.failures.set(event, attempts);
      // Re-head so a transient failure keeps its ordering, but give up the
      // head once it looks permanent and there is other work waiting — one
      // unregisterable query must not stall every other query's registration.
      const starvingOthers = attempts >= MAX_HEAD_RETRIES && this.queue.length > 0;
      if (starvingOthers) {
        this.queue.push(event);
      } else {
        this.queue.unshift(event);
      }
      this.logger.error(
        { error, event, attempts, rotated: starvingOthers, Category: 'sp00ky-client::DownQueue::next' },
        'Failed to process query'
      );
      return error;
    }
  }
}
