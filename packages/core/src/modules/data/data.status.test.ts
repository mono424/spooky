import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState, QueryStatus } from '../../types';

/**
 * Tests for the per-query fetch status (idle/fetching) added to DataModule:
 * setQueryStatus updates the query object and notifies both the DevTools
 * observer hook and subscribeStatus listeners; beginFetching/endFetching
 * refcount overlapping fetch cycles; flushPendingStreamUpdate lands the
 * debounced result before a query flips back to idle.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

function makeDataModule(): DataModule<any> {
  return new DataModule(
    {} as any, // cache — unused by status methods
    {} as any, // local — unused by status methods
    { tables: [] } as any, // schema — unused by status methods
    makeLogger(),
    100
  );
}

function makeQueryState(hash: string): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM user',
      params: {},
      localArray: [],
      remoteArray: [],
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'user',
    },
    records: [],
    ttlTimer: null,
    ttlDurationMs: 0,
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

describe('DataModule query fetch status', () => {
  let dm: DataModule<any>;
  const hash = 'h1';

  beforeEach(() => {
    dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
  });

  it('subscribeStatus with immediate reports the current status', () => {
    const seen: QueryStatus[] = [];
    dm.subscribeStatus(hash, (s) => seen.push(s), { immediate: true });
    expect(seen).toEqual(['idle']);
  });

  it('setQueryStatus updates the query object and notifies subscribers + observer', () => {
    const observed: Array<[string, QueryStatus]> = [];
    dm.onQueryStatusChange = (h, s) => observed.push([h, s]);

    const seen: QueryStatus[] = [];
    dm.subscribeStatus(hash, (s) => seen.push(s));

    dm.setQueryStatus(hash, 'fetching');
    expect((dm as any).activeQueries.get(hash).status).toBe('fetching');
    expect(seen).toEqual(['fetching']);
    expect(observed).toEqual([[hash, 'fetching']]);

    dm.setQueryStatus(hash, 'idle');
    expect(seen).toEqual(['fetching', 'idle']);
    expect(observed).toEqual([[hash, 'fetching'], [hash, 'idle']]);
  });

  it('setQueryStatus is a no-op when the status is unchanged', () => {
    const seen: QueryStatus[] = [];
    dm.subscribeStatus(hash, (s) => seen.push(s));
    dm.setQueryStatus(hash, 'idle'); // already idle
    expect(seen).toEqual([]);
  });

  it('setQueryStatus is a no-op for an unknown query', () => {
    let called = false;
    dm.onQueryStatusChange = () => {
      called = true;
    };
    dm.setQueryStatus('does-not-exist', 'fetching');
    expect(called).toBe(false);
  });

  it('unsubscribe stops further status notifications', () => {
    const seen: QueryStatus[] = [];
    const unsub = dm.subscribeStatus(hash, (s) => seen.push(s));
    dm.setQueryStatus(hash, 'fetching');
    unsub();
    dm.setQueryStatus(hash, 'idle');
    expect(seen).toEqual(['fetching']);
  });
});

describe('DataModule beginFetching/endFetching refcount', () => {
  let dm: DataModule<any>;
  const hash = 'h1';
  let seen: QueryStatus[];

  beforeEach(() => {
    dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
    seen = [];
    dm.subscribeStatus(hash, (s) => seen.push(s));
  });

  it('emits fetching on 0→1 and idle only on the last exit', () => {
    dm.beginFetching(hash); // registration
    dm.beginFetching(hash); // overlapping poll round
    expect(seen).toEqual(['fetching']);

    dm.endFetching(hash); // inner cycle finishes — must NOT emit idle
    expect(seen).toEqual(['fetching']);
    expect((dm as any).activeQueries.get(hash).status).toBe('fetching');

    dm.endFetching(hash);
    expect(seen).toEqual(['fetching', 'idle']);
  });

  it('unbalanced endFetching settles to idle without going negative', () => {
    dm.endFetching(hash); // already idle → no-op notification
    expect(seen).toEqual([]);
    dm.beginFetching(hash);
    dm.endFetching(hash);
    expect(seen).toEqual(['fetching', 'idle']);
    expect((dm as any).fetchDepth.has(hash)).toBe(false);
  });
});

describe('DataModule pending stream-update flush', () => {
  let dm: DataModule<any>;
  const hash = 'h1';
  let processed: any[];

  const makeUpdate = (op: 'CREATE' | 'UPDATE' | 'DELETE') =>
    ({ queryHash: hash, op, localArray: [] }) as any;

  beforeEach(() => {
    vi.useFakeTimers();
    dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
    processed = [];
    (dm as any).processStreamUpdate = async (u: any) => {
      processed.push(u);
    };
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('flushPendingStreamUpdate processes the debounced update exactly once', async () => {
    const update = makeUpdate('CREATE');
    await dm.onStreamUpdate(update);
    expect(processed).toEqual([]); // still debounced

    await dm.flushPendingStreamUpdate(hash);
    expect(processed).toEqual([update]);
    expect((dm as any).debounceTimers.has(hash)).toBe(false);
    expect((dm as any).pendingStreamUpdates.has(hash)).toBe(false);

    // The cancelled timer must not process it a second time.
    await vi.runAllTimersAsync();
    expect(processed).toEqual([update]);

    // Nothing pending → flush is a no-op.
    await dm.flushPendingStreamUpdate(hash);
    expect(processed).toEqual([update]);
  });

  it('the trailing-edge timer clears the pending entry itself', async () => {
    const update = makeUpdate('UPDATE');
    await dm.onStreamUpdate(update);
    await vi.runAllTimersAsync();
    expect(processed).toEqual([update]);
    expect((dm as any).pendingStreamUpdates.has(hash)).toBe(false);
  });

  it('a DELETE supersedes and drops the pending coalesced update', async () => {
    const create = makeUpdate('CREATE');
    const del = makeUpdate('DELETE');
    await dm.onStreamUpdate(create);
    await dm.onStreamUpdate(del); // immediate, drops the pending CREATE
    expect(processed).toEqual([del]);
    expect((dm as any).pendingStreamUpdates.has(hash)).toBe(false);
    await vi.runAllTimersAsync();
    expect(processed).toEqual([del]);
  });

  it('finalizeDeregister clears pending update and fetch depth', async () => {
    (dm as any).cache = { unregisterQuery: () => {} };
    await dm.onStreamUpdate(makeUpdate('CREATE'));
    dm.beginFetching(hash);
    dm.finalizeDeregister(hash);
    expect((dm as any).pendingStreamUpdates.has(hash)).toBe(false);
    expect((dm as any).debounceTimers.has(hash)).toBe(false);
    expect((dm as any).fetchDepth.has(hash)).toBe(false);
    await vi.runAllTimersAsync();
    expect(processed).toEqual([]);
  });
});

describe('DataModule notifyQuerySynced', () => {
  const hash = 'h1';

  function makeDmWithLocal(rows: any[]): DataModule<any> {
    const local: any = { query: async () => [rows] };
    return new DataModule({} as any, local, { tables: [] } as any, makeLogger(), 100);
  }

  it('notifies once per registration lifetime even when records are unchanged and updateCount > 0', async () => {
    const dm = makeDmWithLocal([]);
    const state = makeQueryState(hash);
    state.updateCount = 7; // persisted from a previous registration
    (dm as any).activeQueries.set(hash, state);

    let notified = 0;
    dm.subscribe(hash, () => {
      notified++;
    });

    await dm.notifyQuerySynced(hash); // empty result, unchanged — must still emit
    expect(notified).toBe(1);

    await dm.notifyQuerySynced(hash); // already notified this lifetime → silent
    expect(notified).toBe(1);
  });
});
