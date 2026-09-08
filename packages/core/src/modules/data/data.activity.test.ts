import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState } from '../../types';

/**
 * The aggregate fetch-activity channel (`subscribeActivity`): one number, how
 * many queries are inside their outermost fetch cycle, so an app-chrome
 * "downloading" indicator needs a single subscription instead of one per query.
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

describe('DataModule fetch activity', () => {
  it('emits the current count immediately and on every change', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set('a', makeQueryState('a'));
    (dm as any).activeQueries.set('b', makeQueryState('b'));
    const seen: number[] = [];
    dm.subscribeActivity((n) => seen.push(n), { immediate: true });
    expect(seen).toEqual([0]);

    dm.beginFetching('a');
    dm.beginFetching('b');
    expect(seen).toEqual([0, 1, 2]);
    expect(dm.fetchingQueryCount).toBe(2);

    dm.endFetching('a');
    dm.endFetching('b');
    expect(seen).toEqual([0, 1, 2, 1, 0]);
  });

  it('counts a query once however deep its fetch cycles nest', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set('a', makeQueryState('a'));
    const seen: number[] = [];
    dm.subscribeActivity((n) => seen.push(n));
    dm.beginFetching('a');
    dm.beginFetching('a'); // overlapping poll round on the same hash
    dm.endFetching('a');
    expect(seen).toEqual([1], 'the inner cycle is silent');
    dm.endFetching('a');
    expect(seen).toEqual([1, 0]);
  });

  it('stops notifying after unsubscribe and survives a throwing subscriber', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set('a', makeQueryState('a'));
    const bad = vi.fn(() => {
      throw new Error('boom');
    });
    const good = vi.fn();
    dm.subscribeActivity(bad);
    const off = dm.subscribeActivity(good);
    dm.beginFetching('a');
    expect(good).toHaveBeenCalledWith(1);
    off();
    dm.endFetching('a');
    expect(good).toHaveBeenCalledTimes(1);
    expect(bad).toHaveBeenCalledTimes(2);
  });

  it('drops to zero when a bucket switch quiesces in-flight fetches', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set('a', makeQueryState('a'));
    const seen: number[] = [];
    dm.subscribeActivity((n) => seen.push(n));
    dm.beginFetching('a');
    dm.quiesce();
    expect(seen).toEqual([1, 0]);
    expect(dm.fetchingQueryCount).toBe(0);
  });
});
