import { describe, it, expect, beforeEach, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState } from '../../types';

/**
 * Tests for the local-bucket-switch surface of DataModule:
 * - `quiesce()` disarms every debounce + TTL timer;
 * - `rebindAfterBucketSwitch()` keeps query hashes (subscriptions stay
 *   attached), resets sync state, notifies subscribers with the emptied
 *   records so the previous user's rows leave the UI, re-registers the SSP
 *   view, and returns the hashes for remote re-registration;
 * - stale-epoch stream updates are dropped instead of applied.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

function makeQueryState(hash: string, records: Record<string, any>[]): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM user',
      params: {},
      localArray: [['user:a', 1]],
      remoteArray: [['user:a', 1]],
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'user',
    },
    records,
    hydrated: true,
    ttlTimer: null,
    ttlDurationMs: 600_000,
    updateCount: 3,
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

function makeHarness(epoch = 0) {
  const localQueries: Array<{ query: unknown; vars: unknown; opts: unknown }> = [];
  const local: any = {
    epoch,
    query: vi.fn(async (query: unknown, vars?: unknown, opts?: unknown) => {
      localQueries.push({ query, vars, opts });
      return [[]];
    }),
  };
  const cache: any = {
    registerQuery: vi.fn(() => ({ localArray: [] })),
  };
  const dm = new DataModule(cache, local, { tables: [] } as any, makeLogger(), 100);
  return { dm, local, cache, localQueries };
}

describe('DataModule.quiesce', () => {
  it('clears debounce and TTL timers', () => {
    vi.useFakeTimers();
    try {
      const { dm } = makeHarness();
      const qs = makeQueryState('h1', []);
      qs.ttlTimer = setTimeout(() => {}, 60_000);
      (dm as any).activeQueries.set('h1', qs);
      (dm as any).debounceTimers.set('h1', setTimeout(() => {}, 60_000));

      dm.quiesce();

      expect(qs.ttlTimer).toBeNull();
      expect((dm as any).debounceTimers.size).toBe(0);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('DataModule.rebindAfterBucketSwitch', () => {
  let harness: ReturnType<typeof makeHarness>;

  beforeEach(() => {
    harness = makeHarness();
    (harness.dm as any).activeQueries.set(
      'h1',
      makeQueryState('h1', [{ id: 'user:a', name: 'Previous User Row' }])
    );
  });

  it('keeps the hash, resets sync state, and notifies subscribers with empty records', async () => {
    const { dm, cache } = harness;
    const emissions: unknown[][] = [];
    dm.subscribe('h1', (records) => emissions.push(records as unknown[]));

    const hashes = await dm.rebindAfterBucketSwitch();

    expect(hashes).toEqual(['h1']);
    const qs = (dm as any).activeQueries.get('h1') as QueryState;
    expect(qs.records).toEqual([]);
    expect(qs.config.localArray).toEqual([]);
    expect(qs.config.remoteArray).toEqual([]);
    expect(qs.hydrated).toBe(false);
    expect(qs.status).toBe('fetching');
    // Subscribers saw the previous user's rows drop out.
    expect(emissions).toEqual([[]]);
    // The SSP view was re-registered (on the fresh, post-reset processor).
    expect(cache.registerQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryHash: 'h1', surql: 'SELECT * FROM user' })
    );
    // The TTL heartbeat is re-armed.
    expect(qs.ttlTimer).not.toBeNull();
    if (qs.ttlTimer) clearTimeout(qs.ttlTimer);
  });
});

describe('DataModule.rebindAfterBucketSwitch durable seed', () => {
  it('seeds a confirmed-empty membership from the new bucket, and ignores an unconfirmed one', async () => {
    const harness = makeHarness();
    const { dm, local } = harness as any;
    const state = makeQueryState('h1', [{ id: 'user:a', name: 'Previous User Row' }]);
    state.config.membershipKey = 'stable-key';
    (dm as any).activeQueries.set('h1', state);
    local.getById = vi.fn(async () => ({ ids: [], confirmed: true }));

    await dm.rebindAfterBucketSwitch();
    let qs = (dm as any).activeQueries.get('h1') as QueryState;
    expect(qs.config.membershipKnown).toBe(true);
    expect(qs.config.remoteArray).toEqual([]);
    if (qs.ttlTimer) clearTimeout(qs.ttlTimer);

    local.getById = vi.fn(async () => ({ ids: [] }));
    await dm.rebindAfterBucketSwitch();
    qs = (dm as any).activeQueries.get('h1') as QueryState;
    expect(qs.config.membershipKnown).toBe(false);
    if (qs.ttlTimer) clearTimeout(qs.ttlTimer);
  });
});

describe('stale-epoch stream updates', () => {
  it('drops an update whose chain started before a bucket switch', async () => {
    const { dm, local } = makeHarness();
    const qs = makeQueryState('h1', [{ id: 'user:a' }]);
    (dm as any).activeQueries.set('h1', qs);

    // The materialize read happens, then the epoch moves (bucket switched).
    local.query.mockImplementation(async () => {
      local.epoch = 1;
      return [[{ id: 'user:b', name: 'other user row' }]];
    });

    await (dm as any).processStreamUpdate({
      queryHash: 'h1',
      localArray: [['user:b', 1]],
      op: 'CREATE',
    });

    // Neither the records nor the persisted arrays moved.
    expect(qs.records).toEqual([{ id: 'user:a' }]);
    expect(qs.config.localArray).toEqual([['user:a', 1]]);
  });
});
