import type { Logger } from '../../services/logger/index';
import type { UpQueue, DownQueue, DownEvent, UpEvent, RollbackCallback } from './queue/index';
import { SyncQueueEventTypes } from './events/index';

/**
 * SyncScheduler manages when to sync: queue management and orchestration.
 * Decides the order and timing of sync operations.
 */
/** Backoff for re-draining a queue that halted on an error. */
const RETRY_BASE_MS = 500;
const RETRY_MAX_MS = 15_000;

export class SyncScheduler {
  private isSyncingUp: boolean = false;
  private isSyncingDown: boolean = false;
  private paused: boolean = false;
  private pauseWaiters: Array<() => void> = [];
  // A failed drain re-queues its item at the HEAD (see DownQueue.next) and
  // stops the pass. Without a timer nothing ever drains it again: the queues
  // only move on a fresh enqueue, so a transient failure — canonically the
  // SSP answering 503 NOT_READY for the whole of its bootstrap window — left
  // every pending `register` parked forever and its `useQuery` loading forever.
  // Retry on a backoff so that heals itself instead of needing a reload.
  private upRetryTimer?: ReturnType<typeof setTimeout>;
  private downRetryTimer?: ReturnType<typeof setTimeout>;
  private upRetryAttempt = 0;
  private downRetryAttempt = 0;

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
    private onSyncOutcome?: (ok: boolean, error?: unknown) => void,
    // Reports each mutation the server accepted, once its outbox row is gone.
    // Lets the consumer keep the row rendered until its membership arrives.
    private onSettled?: (event: UpEvent) => void
  ) {}

  async init(opts: { loadOutbox?: boolean } = {}) {
    // Shared-tabs followers skip the outbox load: the shared store's
    // `_00_pending_mutations` is drained by exactly one tab (the leader).
    if (opts.loadOutbox !== false) await this.upQueue.loadFromDatabase();
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
   * Suspend syncing for a local-bucket switch. Refuses new rounds and resolves
   * once any in-flight round has finished — the pause point is BETWEEN queue
   * items, never between an item's remote push and its outbox-row delete, so a
   * processed mutation's `DELETE _00_pending_mutations` always lands in the
   * store it was read from.
   */
  pause(): Promise<void> {
    this.paused = true;
    this.clearRetryTimers();
    if (!this.isSyncingUp && !this.isSyncingDown) return Promise.resolve();
    return new Promise<void>((resolve) => this.pauseWaiters.push(resolve));
  }

  resume(): void {
    this.paused = false;
    // A resume is a fresh start, not a continuation of the failing streak.
    this.upRetryAttempt = 0;
    this.downRetryAttempt = 0;
    void this.syncUp();
    void this.syncDown();
  }

  private maybeResolvePause() {
    if (!this.paused || this.isSyncingUp || this.isSyncingDown) return;
    const waiters = this.pauseWaiters;
    this.pauseWaiters = [];
    for (const resolve of waiters) resolve();
  }

  /** Exponential backoff, capped. Attempt 0 is the first retry. */
  private retryDelay(attempt: number): number {
    return Math.min(RETRY_BASE_MS * 2 ** attempt, RETRY_MAX_MS);
  }

  private scheduleUpRetry() {
    if (this.paused || this.upRetryTimer || this.upQueue.size === 0) return;
    const delay = this.retryDelay(this.upRetryAttempt++);
    this.upRetryTimer = setTimeout(() => {
      this.upRetryTimer = undefined;
      void this.syncUp();
    }, delay);
  }

  /**
   * Re-arm the down pass. With no argument this is failure backoff and the
   * streak grows; with an explicit delay it is a yield (the up-queue holds the
   * floor), which is not a failure and must not push the backoff out.
   */
  private scheduleDownRetry(delayMs?: number) {
    if (this.paused || this.downRetryTimer || this.downQueue.size === 0) return;
    const delay = delayMs ?? this.retryDelay(this.downRetryAttempt++);
    this.downRetryTimer = setTimeout(() => {
      this.downRetryTimer = undefined;
      void this.syncDown();
    }, delay);
  }

  private clearRetryTimers() {
    if (this.upRetryTimer) {
      clearTimeout(this.upRetryTimer);
      this.upRetryTimer = undefined;
    }
    if (this.downRetryTimer) {
      clearTimeout(this.downRetryTimer);
      this.downRetryTimer = undefined;
    }
  }

  /** Stop all pending retries. Call when tearing the client down. */
  dispose(): void {
    this.paused = true;
    this.clearRetryTimers();
  }

  /**
   * Process upload queue
   */
  async syncUp() {
    if (this.isSyncingUp || this.paused) return;
    this.isSyncingUp = true;
    let processedAny = false;
    try {
      while (this.upQueue.size > 0 && !this.paused) {
        await this.upQueue.next(this.onProcessUp, this.onRollback, this.onSettled);
        processedAny = true;
      }
      if (processedAny) this.onSyncOutcome?.(true);
      this.upRetryAttempt = 0;
    } catch (error) {
      this.onSyncOutcome?.(false, error);
      this.scheduleUpRetry();
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
      this.maybeResolvePause();
      void this.syncDown();
    }
  }

  /**
   * Process download queue
   */
  async syncDown() {
    if (this.isSyncingDown || this.paused) return;
    // Down-sync yields to a non-empty up-queue so a register never races ahead
    // of the mutation it should observe. That yield used to be permanent: if
    // the up-queue never drained, nothing re-armed the down pass. Come back on
    // the backoff instead, so a wedged push delays reads rather than killing
    // them.
    if (this.upQueue.size > 0) {
      this.scheduleDownRetry(RETRY_BASE_MS);
      return;
    }

    this.isSyncingDown = true;
    let processedAny = false;
    try {
      while (this.downQueue.size > 0 && !this.paused) {
        if (this.upQueue.size > 0) break;
        await this.downQueue.next(this.onProcessDown);
        processedAny = true;
      }
      if (processedAny) this.onSyncOutcome?.(true);
      this.downRetryAttempt = 0;
    } catch (error) {
      this.onSyncOutcome?.(false, error);
      this.scheduleDownRetry();
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
      this.maybeResolvePause();
    }
  }

  get isSyncing() {
    return this.isSyncingUp || this.isSyncingDown;
  }
}
