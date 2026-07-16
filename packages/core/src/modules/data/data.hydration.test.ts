import { describe, it, expect } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState } from '../../types';

/**
 * Tests for instant-hydrate's DataModule half: `applyHydration` (run-once,
 * remoteArray priming, subscriber notify, and the epoch guard added when the
 * hydrate fetch moved off the paint path into a background chain) and
 * `isPreloadFresh` (the marker check that lets a freshly-preloaded query skip
 * the duplicate one-shot fetch entirely).
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
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
    status: 'fetching',
    phaseSamples: {},
    phaseLast: {},
    registrationTimings: { parseMs: null, planMs: null, snapshotMs: null, wallMs: null },
  };
}

const schema = { tables: [{ name: 'user', columns: {} }] } as any;

function makeRow(id: string, rv = 1) {
  return { id: new RecordId('user', id), name: id, _00_rv: rv };
}

describe('DataModule.applyHydration', () => {
  const hash = 'h1';

  function setup({ epochFlipsOnSave = false } = {}) {
    let epoch = 1;
    const saved: any[] = [];
    const cache: any = {
      saveBatch: async (batch: any[]) => {
        saved.push(...batch);
        if (epochFlipsOnSave) epoch = 2;
      },
    };
    const local: any = {
      get epoch() {
        return epoch;
      },
      query: async () => [[makeRow('a')]],
    };
    const dm = new DataModule(cache, local, schema, makeLogger(), 100);
    const state = makeQueryState(hash);
    (dm as any).activeQueries.set(hash, state);
    return { dm, state, saved };
  }

  it('primes remoteArray, materializes and notifies subscribers', async () => {
    const { dm, state, saved } = setup();
    const emissions: any[] = [];
    dm.subscribe(hash, (records) => emissions.push(records));

    await dm.applyHydration(hash, [makeRow('a', 3)]);

    expect(state.hydrated).toBe(true);
    expect(state.config.remoteArray).toEqual([['user:a', 3]]);
    expect(saved.length).toBe(1);
    expect(emissions.length).toBe(1);
    expect(dm.isCold(hash)).toBe(false);
  });

  it('runs once even when the remote returns nothing (hydrated flag)', async () => {
    const { dm, state, saved } = setup();
    await dm.applyHydration(hash, []);
    expect(state.hydrated).toBe(true);
    expect(saved).toEqual([]);
    expect(dm.isCold(hash)).toBe(false); // hydrated → no longer cold
  });

  it('is a no-op for an unknown query', async () => {
    const { dm, saved } = setup();
    await dm.applyHydration('nope', [makeRow('a')]);
    expect(saved).toEqual([]);
  });

  it('bails before mutating state when the bucket epoch moves mid-persist', async () => {
    const { dm, state } = setup({ epochFlipsOnSave: true });
    const emissions: any[] = [];
    dm.subscribe(hash, (records) => emissions.push(records));

    await dm.applyHydration(hash, [makeRow('a', 3)]);

    // Rows fetched under the previous auth context must not prime the new
    // bucket's query state — the rebind's re-registration refills it.
    expect(state.config.remoteArray).toEqual([]);
    expect(state.records).toEqual([]);
    expect(emissions).toEqual([]);
  });
});

describe('DataModule.isPreloadFresh', () => {
  const maxAgeMs = 60_000;

  function makeDm(getById: (table: string, id: string) => Promise<any>) {
    const local: any = { getById };
    return new DataModule({} as any, local, schema, makeLogger(), 100);
  }

  it('true for a marker younger than maxAgeMs', async () => {
    const dm = makeDm(async () => ({ fetchedAt: Date.now() - 1_000, rowCount: 5 }));
    expect(await dm.isPreloadFresh('123', maxAgeMs)).toBe(true);
  });

  it('false for a marker older than maxAgeMs', async () => {
    const dm = makeDm(async () => ({ fetchedAt: Date.now() - maxAgeMs - 1, rowCount: 5 }));
    expect(await dm.isPreloadFresh('123', maxAgeMs)).toBe(false);
  });

  it('false when no marker exists', async () => {
    const dm = makeDm(async () => null);
    expect(await dm.isPreloadFresh('123', maxAgeMs)).toBe(false);
  });

  it('false when the marker read throws (treated as cold)', async () => {
    const dm = makeDm(async () => {
      throw new Error('boom');
    });
    expect(await dm.isPreloadFresh('123', maxAgeMs)).toBe(false);
  });
});
