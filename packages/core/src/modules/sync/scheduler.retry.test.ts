import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { SyncScheduler } from './scheduler';
import type { UpQueue, DownQueue, DownEvent, UpEvent } from './queue/index';

// A queue whose drain throws re-queues the failing item at the HEAD and stops
// the pass (see DownQueue.next). Nothing used to re-arm it: the queues only
// moved on a fresh enqueue, so one transient failure — canonically the SSP
// answering 503 NOT_READY for the whole of its bootstrap window — parked every
// pending `register` forever, and the `useQuery` waiting on it never left its
// loading state. These cover the backoff that makes that self-heal.

const silentLogger = {
  child: () => silentLogger,
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: () => {},
} as any;

/** A queue that mirrors the real ones: a throwing handler re-heads the item. */
function makeQueue<E>(items: E[]) {
  return {
    queue: [...items],
    get size() {
      return this.queue.length;
    },
    events: { subscribe: () => {} },
    loadFromDatabase: async () => {},
    clear() {
      this.queue = [];
    },
    async next(fn: (event: E) => Promise<void>) {
      const event = this.queue.shift();
      if (!event) return;
      try {
        await fn(event);
      } catch (err) {
        this.queue.unshift(event);
        throw err;
      }
    },
  };
}

const downEvent = (hash: string) => ({ type: 'register', payload: { hash } }) as DownEvent;
const upEvent = (n: number) => ({ type: 'delete', mutation_id: n, record_id: n }) as unknown as UpEvent;

describe('SyncScheduler retry', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('re-drains a failed down event instead of parking it forever', async () => {
    const downQueue = makeQueue([downEvent('q1')]);
    const upQueue = makeQueue<UpEvent>([]);
    let attempts = 0;
    const scheduler = new SyncScheduler(
      upQueue as unknown as UpQueue,
      downQueue as unknown as DownQueue,
      async () => {},
      async () => {
        attempts++;
        // Fail the way a bootstrapping SSP does, then succeed.
        if (attempts < 3) throw new Error('503 NOT_READY');
      },
      silentLogger
    );

    await scheduler.syncDown();
    expect(attempts).toBe(1);
    expect(downQueue.size).toBe(1); // re-headed, not dropped

    // Backoff: 500ms, then 1000ms.
    await vi.advanceTimersByTimeAsync(500);
    expect(attempts).toBe(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts).toBe(3);

    expect(downQueue.size).toBe(0);
  });

  it('stops retrying once the queue drains', async () => {
    const downQueue = makeQueue([downEvent('q1')]);
    const upQueue = makeQueue<UpEvent>([]);
    let attempts = 0;
    const scheduler = new SyncScheduler(
      upQueue as unknown as UpQueue,
      downQueue as unknown as DownQueue,
      async () => {},
      async () => {
        attempts++;
        throw new Error('boom');
      },
      silentLogger
    );

    await scheduler.syncDown();
    await vi.advanceTimersByTimeAsync(500);
    expect(attempts).toBe(2);

    // Drain it out from under the scheduler; the next retry finds nothing and
    // schedules no further work.
    downQueue.clear();
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts).toBe(2);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(attempts).toBe(2);
  });

  it('comes back for the down queue while the up queue holds the floor', async () => {
    const downQueue = makeQueue([downEvent('q1')]);
    const upQueue = makeQueue<UpEvent>([upEvent(1)]);
    let downAttempts = 0;
    const scheduler = new SyncScheduler(
      upQueue as unknown as UpQueue,
      downQueue as unknown as DownQueue,
      async () => {},
      async () => {
        downAttempts++;
      },
      silentLogger
    );

    // Yields to the non-empty up queue.
    await scheduler.syncDown();
    expect(downAttempts).toBe(0);

    // Once the up queue empties, the re-armed pass picks the down event up
    // without needing a fresh enqueue.
    upQueue.clear();
    await vi.advanceTimersByTimeAsync(500);
    expect(downAttempts).toBe(1);
  });

  it('pause cancels pending retries', async () => {
    const downQueue = makeQueue([downEvent('q1')]);
    const upQueue = makeQueue<UpEvent>([]);
    let attempts = 0;
    const scheduler = new SyncScheduler(
      upQueue as unknown as UpQueue,
      downQueue as unknown as DownQueue,
      async () => {},
      async () => {
        attempts++;
        throw new Error('boom');
      },
      silentLogger
    );

    await scheduler.syncDown();
    expect(attempts).toBe(1);

    await scheduler.pause();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(attempts).toBe(1);
  });
});
