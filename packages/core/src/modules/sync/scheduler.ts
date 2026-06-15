import type { Logger } from '../../services/logger/index';
import type { UpQueue, DownQueue, DownEvent, UpEvent, RollbackCallback } from './queue/index';
import { SyncQueueEventTypes } from './events/index';

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
    private logger: Logger,
    private onRollback?: RollbackCallback,
    // Reports the outcome of each drained sync round (one syncUp/syncDown pass
    // that actually processed ≥1 item): `ok=true` on a clean drain, `ok=false`
    // with the error when the round halted on a failure. Drives the consumer's
    // sync-health tracking; empty/no-op rounds report nothing.
    private onSyncOutcome?: (ok: boolean, error?: unknown) => void
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
    let processedAny = false;
    try {
      while (this.upQueue.size > 0) {
        await this.upQueue.next(this.onProcessUp, this.onRollback);
        processedAny = true;
      }
      if (processedAny) this.onSyncOutcome?.(true);
    } catch (error) {
      this.onSyncOutcome?.(false, error);
      // syncUp runs fire-and-forget — it's wired to the MutationEnqueued event
      // (broadcast synchronously, return value dropped) and is also kicked off
      // via `void this.syncDown()` below. A rejection escaping here therefore
      // surfaces as an *unhandled promise rejection* in the console rather than
      // anything a caller can catch. UpQueue.next already logs the failing item
      // (and re-queues it for retry on the next trigger), so swallow here to
      // keep the failure contained instead of leaking it globally.
      this.logger.debug(
        { error, Category: 'sp00ky-client::SyncScheduler::syncUp' },
        'syncUp halted on a queue error; item re-queued, will retry on next trigger'
      );
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
    let processedAny = false;
    try {
      while (this.downQueue.size > 0) {
        if (this.upQueue.size > 0) break;
        await this.downQueue.next(this.onProcessDown);
        processedAny = true;
      }
      if (processedAny) this.onSyncOutcome?.(true);
    } catch (error) {
      this.onSyncOutcome?.(false, error);
      // Same fire-and-forget story as syncUp: this is the QueryItemEnqueued
      // subscriber (and is also called via `void this.syncDown()`), so a thrown
      // error here becomes an unhandled rejection. The canonical case is a
      // transient remote 500 on `fn::query::register` — DownQueue.next logs it
      // and re-queues the event at the head; we just stop draining this pass and
      // let the next enqueue retry, without spamming the console with an
      // "Uncaught (in promise) ... 500 Internal Server Error".
      this.logger.debug(
        { error, Category: 'sp00ky-client::SyncScheduler::syncDown' },
        'syncDown halted on a queue error; item re-queued, will retry on next trigger'
      );
    } finally {
      this.isSyncingDown = false;
    }
  }

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }
}
