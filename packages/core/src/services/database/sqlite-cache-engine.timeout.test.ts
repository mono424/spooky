import { describe, it, expect, vi, afterEach } from 'vitest';
import { SqliteCacheEngine } from './sqlite-cache-engine';
import { stubTransport } from './sqlite-transport.fixture';
import { LocalOpTimeoutError } from './errors';
import { withRetry } from '../../utils/index';

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

/**
 * The worker transport parks a call until the worker replies. A worker that
 * never did (starved, or wedged on an unbounded lock check) left every caller
 * waiting forever. Each round trip now has a deadline.
 */
describe('SqliteCacheEngine local op deadline', () => {
  afterEach(() => vi.useRealTimers());

  async function openEngine(handler: (type: string, payload: any) => unknown) {
    const engine = new SqliteCacheEngine(
      { namespace: 'n', database: 'd', localOpTimeoutMs: 50 } as any,
      makeLogger()
    );
    stubTransport(engine, handler);
    await engine.connect('bucket-a');
    return engine;
  }

  it('rejects a call that never answers with LocalOpTimeoutError and keeps the queue moving', async () => {
    vi.useFakeTimers();
    let hang = false;
    const engine = await openEngine((type) => {
      if (hang && type !== 'open') return new Promise(() => {});
      return { rows: [], result: [] };
    });
    hang = true;
    const stuck = engine.query('SELECT * FROM game');
    const settled = stuck.then(
      () => 'resolved',
      (e) => e
    );
    await vi.advanceTimersByTimeAsync(60);
    const err = await settled;
    expect(err).toBeInstanceOf(LocalOpTimeoutError);
    expect((err as Error).message).toMatch(/timed out/);
    // The next op runs; the transport is not torn down for a slow worker.
    hang = false;
    await expect(engine.query('SELECT * FROM game')).resolves.toBeDefined();
  });

  it('is not retried by withRetry', async () => {
    const attempts = vi.fn(async () => {
      throw new LocalOpTimeoutError('run', 5);
    });
    await expect(withRetry(makeLogger(), attempts)).rejects.toBeInstanceOf(LocalOpTimeoutError);
    expect(attempts).toHaveBeenCalledTimes(1);
  });
});
