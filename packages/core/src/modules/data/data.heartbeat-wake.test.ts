import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState } from '../../types';

/**
 * The TTL heartbeat is the only thing keeping a view alive on the server, and
 * browsers freeze timers in hidden tabs. `heartbeatOnWake` re-asserts liveness
 * for every WATCHED query the moment the tab is back (visibilitychange /
 * online) and re-anchors its timer. There is no client-side expiry anywhere:
 * the TTL belongs to the server.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

function makeDataModule(): DataModule<any> {
  return new DataModule({} as any, {} as any, { tables: [] } as any, makeLogger(), 100);
}

const TTL_MS = 600_000;

function makeQueryState(hash: string): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM user',
      params: {},
      localArray: [],
      remoteArray: [['user:a', 1]],
      membershipKnown: true,
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'user',
    },
    records: [{ id: 'user:a' }],
    ttlTimer: null,
    ttlDurationMs: TTL_MS,
    updateCount: 0,
    lastUpdatedAt: null,
    materializationSamples: [],
    lastIngestLatencyMs: null,
    errorCount: 0,
    status: 'idle',
    phaseSamples: {},
    phaseLast: {},
    registrationTimings: { parseMs: null, planMs: null, snapshotMs: null, wallMs: null },
  };
}

function setup() {
  const dm = makeDataModule();
  const onHeartbeat = vi.fn();
  dm.onHeartbeat = onHeartbeat;
  const watched = makeQueryState('watched');
  const idle = makeQueryState('idle');
  (dm as any).activeQueries.set('watched', watched);
  (dm as any).activeQueries.set('idle', idle);
  (dm as any).subscriptions.set('watched', new Set([() => {}]));
  (dm as any).startTTLHeartbeat(watched, 'watched');
  (dm as any).startTTLHeartbeat(idle, 'idle');
  return { dm, onHeartbeat, watched, idle };
}

describe('DataModule heartbeat on wake', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('beats every watched query now and re-anchors its timer', async () => {
    const { dm, onHeartbeat, watched } = setup();
    await vi.advanceTimersByTimeAsync(200_000);

    expect(dm.heartbeatOnWake()).toBe(1);
    expect(onHeartbeat).toHaveBeenCalledTimes(1);
    expect(onHeartbeat).toHaveBeenCalledWith('watched');
    expect(watched.ttlTimer).not.toBeNull();

    // The regular beat would have been due at 300s; it is now due 300s after
    // the wake beat instead.
    await vi.advanceTimersByTimeAsync(TTL_MS / 2 - 1_000);
    expect(onHeartbeat).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(onHeartbeat).toHaveBeenCalledTimes(2);
  });

  it('skips queries nobody is watching', () => {
    const { dm, onHeartbeat } = setup();
    dm.heartbeatOnWake();
    expect(onHeartbeat).not.toHaveBeenCalledWith('idle');
  });

  it('does not beat the same query twice within the gap', async () => {
    const { dm, onHeartbeat } = setup();
    expect(dm.heartbeatOnWake()).toBe(1);
    expect(dm.heartbeatOnWake()).toBe(0);
    await vi.advanceTimersByTimeAsync(31_000);
    expect(dm.heartbeatOnWake()).toBe(1);
    expect(onHeartbeat).toHaveBeenCalledTimes(2);
  });

  it('never expires local query state on its own', async () => {
    const { dm, watched, idle } = setup();
    // Well past the TTL, with no server in the picture.
    await vi.advanceTimersByTimeAsync(TTL_MS * 3);
    expect((dm as any).activeQueries.get('watched')).toBe(watched);
    expect((dm as any).activeQueries.get('idle')).toBe(idle);
    expect(watched.records).toEqual([{ id: 'user:a' }]);
    expect(watched.config.remoteArray).toEqual([['user:a', 1]]);
    expect(watched.config.membershipKnown).toBe(true);
  });
});
