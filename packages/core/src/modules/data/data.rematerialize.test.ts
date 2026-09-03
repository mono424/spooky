import { describe, it, expect, vi, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryPlan } from '@spooky-sync/query-builder';
import type { QueryState, RecordVersionArray } from '../../types';

/**
 * A row this client wrote itself reaches server membership at the very
 * version the local CREATE memoized, so the sync engine rightly fetches
 * nothing when the membership lands - and then nothing re-materialized the
 * query: `remoteArray` gained the id and no subscriber heard about it until a
 * reload rebuilt the view from membership. `scheduleRematerialize` closes that
 * gap through the same per-query debounce a real stream update uses.
 */

const noop = () => {};
function makeLogger(): any {
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

const schema = { tables: [{ name: 'message', columns: {} }] } as any;
const plan: QueryPlan = { table: 'message', where: [['conversation', '=', 'conversation:c1']] } as any;

function makeQueryState(hash: string, remoteArray: RecordVersionArray, localArray: RecordVersionArray): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM message WHERE conversation = $conversation;',
      plan,
      params: { conversation: 'conversation:c1' },
      localArray,
      remoteArray,
      membershipKnown: true,
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'message',
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
  } as QueryState;
}

function setup(remoteArray: RecordVersionArray, localArray: RecordVersionArray) {
  const body = { id: new RecordId('message', 'm1'), text: 'hi' };
  const local: any = {
    epoch: 1,
    select: vi.fn(async (p: QueryPlan) => {
      const ids = ((p as any).ids as RecordId[] | undefined) ?? [];
      return ids.some((id) => id.toString() === 'message:m1') ? [body] : [];
    }),
    query: vi.fn(async (sql: string) => {
      if (sql.includes('_00_pending_mutations')) return [[]];
      return [[]];
    }),
  };
  const dm = new DataModule({} as any, local, schema, makeLogger(), 10);
  const hash = 'q1';
  (dm as any).activeQueries.set(hash, makeQueryState(hash, remoteArray, localArray));
  const subscriber = vi.fn();
  (dm as any).subscriptions.set(hash, new Set([subscriber]));
  return { dm, local, hash, subscriber };
}

describe('DataModule.scheduleRematerialize', () => {
  afterEach(() => vi.useRealTimers());

  it('notifies subscribers with the row once membership holds it', async () => {
    vi.useFakeTimers();
    const { dm, local, hash, subscriber } = setup([['message:m1', 1]], [['message:m1', 1]]);
    dm.scheduleRematerialize(hash);
    await vi.advanceTimersByTimeAsync(20);
    expect(subscriber).toHaveBeenCalledTimes(1);
    expect(subscriber.mock.calls[0]![0].map((r: any) => r.id.toString())).toEqual(['message:m1']);
    // A synthetic update describes no ingest: nothing is persisted to _00_query.
    expect(local.query.mock.calls.some(([sql]: [string]) => sql.includes('localArray'))).toBe(false);
  });

  it('is a no-op behind a pending real stream update', async () => {
    vi.useFakeTimers();
    const { dm, hash, subscriber } = setup([['message:m1', 1]], [['message:m1', 1]]);
    await dm.onStreamUpdate({ queryHash: hash, localArray: [['message:m1', 1]], op: 'CREATE' });
    dm.scheduleRematerialize(hash);
    await vi.advanceTimersByTimeAsync(20);
    // Exactly one notify: the real update's, not two.
    expect(subscriber).toHaveBeenCalledTimes(1);
  });

  it('does not notify when the records are unchanged', async () => {
    vi.useFakeTimers();
    const { dm, hash, subscriber } = setup([['message:m1', 1]], [['message:m1', 1]]);
    dm.scheduleRematerialize(hash);
    await vi.advanceTimersByTimeAsync(20);
    dm.scheduleRematerialize(hash);
    await vi.advanceTimersByTimeAsync(20);
    expect(subscriber).toHaveBeenCalledTimes(1);
  });

  it('ignores an unknown query', () => {
    const { dm } = setup([], []);
    expect(() => dm.scheduleRematerialize('nope')).not.toThrow();
  });
});
