import { describe, it, expect, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState, QueryStatus } from '../../types';

/**
 * Tests for the per-query fetch status (idle/fetching) added to DataModule:
 * setQueryStatus updates the query object and notifies both the DevTools
 * observer hook and subscribeStatus listeners.
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
    materializationSamples: [],
    lastIngestLatencyMs: null,
    errorCount: 0,
    status: 'idle',
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
