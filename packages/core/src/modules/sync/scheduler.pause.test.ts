import { describe, it, expect, vi } from 'vitest';
import { SyncScheduler } from './scheduler';
import type { UpQueue, DownQueue, UpEvent } from './queue/index';

// `SyncScheduler.pause()` is the drain step of a local-bucket switch: it must
// (a) let an IN-FLIGHT queue item finish — including its outbox-row delete,
// which has to land in the old bucket — and only then resolve, and (b) refuse
// to start new rounds until `resume()`. The pause point is between items.

const silentLogger = {
  child: () => silentLogger,
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: () => {},
} as any;

function makeQueues(items: UpEvent[]) {
  const upItems: UpEvent[] = [...items];
  const upQueue = {
    get size() {
      return upItems.length;
    },
    events: { subscribe: () => {} },
    loadFromDatabase: async () => {},
    async next(fn: (event: UpEvent) => Promise<void>) {
      const event = upItems.shift();
      if (event) await fn(event);
    },
  } as unknown as UpQueue;

  let downItems: unknown[] = [];
  const downQueue = {
    get size() {
      return downItems.length;
    },
    events: { subscribe: () => {} },
    async next() {},
    clear() {
      downItems = [];
    },
  } as unknown as DownQueue;

  return { upQueue, downQueue };
}

const upEvent = (n: number) => ({ type: 'delete', mutation_id: n, record_id: n }) as unknown as UpEvent;

describe('SyncScheduler.pause', () => {
  it('waits for the in-flight item (push + outbox delete) before resolving', async () => {
    const { upQueue, downQueue } = makeQueues([upEvent(1), upEvent(2)]);
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const processed: unknown[] = [];
    const scheduler = new SyncScheduler(
      upQueue,
      downQueue,
      async (event) => {
        await gate; // simulate a slow remote push
        processed.push(event);
      },
      async () => {},
      silentLogger
    );

    const round = scheduler.syncUp();
    const pauseDone = vi.fn();
    const pause = scheduler.pause().then(pauseDone);

    // The in-flight item hasn't finished — pause must not have resolved.
    await Promise.resolve();
    expect(pauseDone).not.toHaveBeenCalled();

    release();
    await pause;
    await round;
    // Exactly the in-flight item completed; the second stayed queued.
    expect(processed).toHaveLength(1);
    expect(upQueue.size).toBe(1);
  });

  it('refuses new rounds while paused and drains them on resume', async () => {
    const { upQueue, downQueue } = makeQueues([upEvent(1)]);
    const processed: unknown[] = [];
    const scheduler = new SyncScheduler(
      upQueue,
      downQueue,
      async (event) => {
        processed.push(event);
      },
      async () => {},
      silentLogger
    );

    await scheduler.pause();
    await scheduler.syncUp();
    expect(processed).toHaveLength(0);

    scheduler.resume();
    await vi.waitFor(() => expect(processed).toHaveLength(1));
    expect(upQueue.size).toBe(0);
  });

  it('resolves immediately when nothing is in flight', async () => {
    const { upQueue, downQueue } = makeQueues([]);
    const scheduler = new SyncScheduler(upQueue, downQueue, async () => {}, async () => {}, silentLogger);
    await expect(scheduler.pause()).resolves.toBeUndefined();
  });
});
