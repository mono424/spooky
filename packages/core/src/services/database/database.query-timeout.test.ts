import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { AbstractDatabaseService } from './database';
import { DatabaseEventTypes, createDatabaseEventSystem } from './events/index';
import { classifySyncError } from '../../utils/index';

// Every remote query is serialized through one promise chain (to keep the WASM
// engine's transactions safe), so a call that never settles doesn't just fail —
// it blocks every later query behind it, including the sync poll's own health
// probe. That's how a half-open socket wedges the client while health still
// reports `healthy`: nothing ever throws, so nothing ever degrades.

const silentLogger: any = (() => {
  const l: any = {
    child: () => l,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  return l;
})();

class TestService extends AbstractDatabaseService {
  protected eventType = DatabaseEventTypes.RemoteQuery;
  constructor(client: any, timeoutMs: number, maxConcurrent = 1) {
    super(client, silentLogger, createDatabaseEventSystem());
    this.queryTimeoutMs = timeoutMs;
    this.maxConcurrentQueries = maxConcurrent;
  }
  async connect(): Promise<void> {}
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe('AbstractDatabaseService query timeout', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('rejects a never-settling query and classifies it as a network failure', async () => {
    const client = { query: vi.fn().mockImplementation(() => new Promise(() => {})) };
    const svc = new TestService(client, 1_000);

    const pending = svc.query('SELECT * FROM thread');
    const assertion = expect(pending).rejects.toThrow(/timed out/);
    await vi.advanceTimersByTimeAsync(1_000);
    await assertion;

    // "timed out" in the message is load-bearing: it routes the failure to
    // `network`, which makes the sync queues retry instead of rolling the
    // mutation back as a permanent application error.
    const err = await pending.catch((e) => e);
    expect(classifySyncError(err)).toBe('network');
  });

  it('unblocks the serialized queue so later queries still run', async () => {
    let calls = 0;
    const client = {
      query: vi.fn().mockImplementation(() => {
        calls++;
        // First call hangs forever; the second would never be reached at all if
        // the timeout didn't settle the chain link.
        if (calls === 1) return new Promise(() => {});
        return Promise.resolve([['ok']]);
      }),
    };
    const svc = new TestService(client, 1_000);

    const stuck = svc.query('SELECT 1');
    const next = svc.query('SELECT 2');
    const stuckAssertion = expect(stuck).rejects.toThrow(/timed out/);

    await vi.advanceTimersByTimeAsync(1_000);
    await stuckAssertion;
    await expect(next).resolves.toEqual([['ok']]);
  });

  it('leaves queries unbounded when the timeout is disabled', async () => {
    const client = { query: vi.fn().mockResolvedValue([['ok']]) };
    const svc = new TestService(client, 0);

    await expect(svc.query('SELECT 1')).resolves.toEqual([['ok']]);
    // No timer was armed, so a slow-but-alive query is never cut off.
    expect(vi.getTimerCount()).toBe(0);
  });

  it('keeps statements strictly ordered at the default limit of 1', async () => {
    const first = deferred<unknown[]>();
    const client = { query: vi.fn().mockImplementationOnce(() => first.promise).mockResolvedValue([['second']]) };
    const svc = new TestService(client, 0);

    const a = svc.query('SELECT 1');
    const b = svc.query('SELECT 2');
    await vi.advanceTimersByTimeAsync(0);
    // The second statement has not been handed to the SDK while the first is in flight.
    expect(client.query).toHaveBeenCalledTimes(1);
    first.resolve([['first']]);
    await expect(a).resolves.toEqual([['first']]);
    await expect(b).resolves.toEqual([['second']]);
    expect(client.query).toHaveBeenCalledTimes(2);
  });

  it('runs statements concurrently up to maxConcurrentQueries and queues the rest', async () => {
    const d1 = deferred<unknown[]>();
    const d2 = deferred<unknown[]>();
    const client = {
      query: vi
        .fn()
        .mockImplementationOnce(() => d1.promise)
        .mockImplementationOnce(() => d2.promise)
        .mockResolvedValue([['third']]),
    };
    const svc = new TestService(client, 0, 2);

    const a = svc.query('SELECT 1');
    const b = svc.query('SELECT 2');
    const c = svc.query('SELECT 3');
    await vi.advanceTimersByTimeAsync(0);
    // Two in flight at once; the third waits for a slot.
    expect(client.query).toHaveBeenCalledTimes(2);
    d2.resolve([['second']]);
    await expect(b).resolves.toEqual([['second']]);
    await vi.advanceTimersByTimeAsync(0);
    expect(client.query).toHaveBeenCalledTimes(3);
    d1.resolve([['first']]);
    await expect(a).resolves.toEqual([['first']]);
    await expect(c).resolves.toEqual([['third']]);
  });

  it('releases the slot when a statement fails', async () => {
    const client = { query: vi.fn().mockRejectedValueOnce(new Error('boom')).mockResolvedValue([['ok']]) };
    const svc = new TestService(client, 0);
    await expect(svc.query('SELECT 1')).rejects.toThrow('boom');
    await expect(svc.query('SELECT 2')).resolves.toEqual([['ok']]);
  });
});
