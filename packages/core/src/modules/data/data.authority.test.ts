import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryState } from '../../types';

/**
 * A query's AUTHORITY is whether server membership is known for it. It is what
 * the bindings' `isAuthoritative()` mirrors, and with `hasData` it is what
 * separates "cold, still waiting for the server" from "the server answered and
 * the answer is empty". `setMembershipKnown` is the one writer; it fans out to
 * `subscribeAuthority` listeners and the DevTools hook only on a change.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

function makeDataModule(): DataModule<any> {
  return new DataModule(
    { unregisterQuery: vi.fn() } as any,
    {} as any,
    { tables: [] } as any,
    makeLogger(),
    100
  );
}

function makeQueryState(hash: string, membershipKnown?: boolean): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM user',
      params: {},
      localArray: [],
      remoteArray: [],
      membershipKnown,
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

describe('DataModule query authority', () => {
  const hash = 'h1';

  it('reports the current value immediately, false for an unknown query', () => {
    const dm = makeDataModule();
    const unknown = vi.fn();
    dm.subscribeAuthority('nope', unknown, { immediate: true });
    expect(unknown).toHaveBeenCalledWith(false);

    (dm as any).activeQueries.set(hash, makeQueryState(hash, true));
    const known = vi.fn();
    dm.subscribeAuthority(hash, known, { immediate: true });
    expect(known).toHaveBeenCalledWith(true);
    expect(dm.isAuthoritative(hash)).toBe(true);
    expect(dm.isAuthoritative('nope')).toBe(false);
  });

  it('emits once per transition through the single writer', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
    const cb = vi.fn();
    const hook = vi.fn();
    dm.onQueryAuthorityChange = hook;
    dm.subscribeAuthority(hash, cb);

    (dm as any).setMembershipKnown(hash, true);
    (dm as any).setMembershipKnown(hash, true);
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenLastCalledWith(true);
    expect(hook).toHaveBeenCalledWith(hash, true);

    (dm as any).setMembershipKnown(hash, false);
    expect(cb).toHaveBeenCalledTimes(2);
    expect(cb).toHaveBeenLastCalledWith(false);
    expect(dm.isAuthoritative(hash)).toBe(false);
  });

  it('is a no-op for an unknown query', () => {
    const dm = makeDataModule();
    const hook = vi.fn();
    dm.onQueryAuthorityChange = hook;
    (dm as any).setMembershipKnown('nope', true);
    expect(hook).not.toHaveBeenCalled();
  });

  it('stops delivering after unsubscribe', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
    const cb = vi.fn();
    const off = dm.subscribeAuthority(hash, cb);
    off();
    (dm as any).setMembershipKnown(hash, true);
    expect(cb).not.toHaveBeenCalled();
    expect((dm as any).authoritySubscriptions.has(hash)).toBe(false);
  });

  it('drops the authority subscribers with the query on finalizeDeregister', () => {
    const dm = makeDataModule();
    (dm as any).activeQueries.set(hash, makeQueryState(hash));
    dm.subscribeAuthority(hash, vi.fn());
    dm.finalizeDeregister(hash);
    expect((dm as any).authoritySubscriptions.has(hash)).toBe(false);
  });
});
