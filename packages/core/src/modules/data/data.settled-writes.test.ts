import { describe, it, expect, vi, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryPlan } from '@spooky-sync/query-builder';
import type { QueryState, RecordVersionArray } from '../../types';

/**
 * A write is visible before the server confirms it because it sits in the
 * outbox: the render set is
 * `(membership ∪ (pendingWrites ∩ localArray)) − pendingDeletes`.
 *
 * The outbox row is deleted the instant the push succeeds, but the row does not
 * enter membership until the SSP has ingested it, materialized the view, written
 * the `_00_list_ref` edge, and this client has read that back. In between it is
 * in NEITHER term, so it renders, disappears, and returns.
 *
 * Reported from production as "the comment shows up on every other client in
 * realtime but not on the one that wrote it" — other clients don't blink because
 * a client that never established membership renders from the predicate scan
 * instead. The gap is invisible when the round trip is fast and glaring when the
 * edge path is backed up.
 */

const noop = () => {};

function makeLogger(): any {
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

const schema = { tables: [{ name: 'comment', columns: {} }] } as any;
const plan: QueryPlan = { table: 'comment', where: [['game', '=', 'game:g1']] } as any;

function makeQueryState(
  hash: string,
  opts: { remoteArray?: RecordVersionArray; localArray?: RecordVersionArray } = {}
): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM comment WHERE game = $game;',
      plan,
      params: { game: 'game:g1' },
      localArray: opts.localArray ?? [],
      remoteArray: opts.remoteArray ?? [],
      membershipKnown: true,
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'comment',
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

function setup(
  stateOpts: Parameters<typeof makeQueryState>[1] = {},
  pendingRows: Array<{ recordId: RecordId; mutationType: string }> = []
) {
  const bodies = new Map(
    ['old', 'new'].map((k) => [`comment:${k}`, { id: new RecordId('comment', k), body: k }])
  );
  const local: any = {
    epoch: 1,
    select: vi.fn(async (p: QueryPlan) => {
      if ((p as any).ids) {
        return ((p as any).ids as RecordId[])
          .map((id) => bodies.get(id.toString()))
          .filter((r): r is NonNullable<typeof r> => r !== undefined);
      }
      return Array.from(bodies.values());
    }),
    query: vi.fn(async (sql: string) => {
      if (sql.includes('_00_pending_mutations')) return [pendingRows];
      if (sql.includes('FROM ONLY')) return [null];
      return [Array.from(bodies.values())];
    }),
    getById: vi.fn(async () => null),
    upsert: vi.fn(async () => {}),
  };
  const dm = new DataModule({ saveBatch: async () => {} } as any, local, schema, makeLogger(), 100);
  const state = makeQueryState('h1', stateOpts);
  (dm as any).activeQueries.set('h1', state);
  return { dm, state };
}

const ids = (rows: Array<Record<string, any>>) => rows.map((r) => String(r.id));
const materialize = (dm: DataModule<any>, state: QueryState, ssp?: RecordVersionArray) =>
  (dm as any).materializeRecords(state, ssp) as Promise<Record<string, any>[]>;

describe('settled-write grace window', () => {
  afterEach(() => vi.useRealTimers());

  it('keeps an accepted write rendered while membership has not caught up', async () => {
    // Membership still only knows the old comment; the outbox row for the new
    // one is already gone because the push succeeded.
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
    });

    dm.noteWriteSettled('comment:new', 'create');

    expect(ids(await materialize(dm, state))).toEqual(['comment:new', 'comment:old']);
  });

  it('renders nothing extra for a write the server never accepted', async () => {
    // The rollback path never reports a mutation settled, so a rejected write
    // has no grace at all. This is what keeps the window from showing rows the
    // server refused.
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
    });

    expect(ids(await materialize(dm, state))).toEqual(['comment:old']);
  });

  it('stops granting grace once membership names the row', async () => {
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
    });
    dm.noteWriteSettled('comment:new', 'create');
    await materialize(dm, state);

    // Membership catches up; the id must not be double-counted…
    state.config.remoteArray = [
      ['comment:old', 1],
      ['comment:new', 1],
    ];
    expect(ids(await materialize(dm, state))).toEqual(['comment:new', 'comment:old']);
    expect((dm as any).settledWrites.size).toBe(0);

    // …and once the server later drops it from the window, it goes away rather
    // than being resurrected by a stale grace entry.
    state.config.remoteArray = [['comment:old', 1]];
    expect(ids(await materialize(dm, state))).toEqual(['comment:old']);
  });

  it('expires the grace so a write whose membership never arrives cannot linger', async () => {
    vi.useFakeTimers();
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
    });
    dm.noteWriteSettled('comment:new', 'create');
    expect(ids(await materialize(dm, state))).toContain('comment:new');
    expect(dm.hasSettledWritesPending()).toBe(true);

    // A scheduler drain plus one stalled database commit fit inside the
    // window: at 11 s the row is still the user's own message, not a ghost.
    vi.advanceTimersByTime(11_000);
    expect(ids(await materialize(dm, state))).toContain('comment:new');
    expect(dm.hasSettledWritesPending()).toBe(true);

    vi.advanceTimersByTime(20_000);
    expect(ids(await materialize(dm, state))).toEqual(['comment:old']);
    expect(dm.hasSettledWritesPending()).toBe(false);
  });

  it('reports no pending settled write once membership has caught up', async () => {
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
    });
    expect(dm.hasSettledWritesPending()).toBe(false);

    dm.noteWriteSettled('comment:new', 'create');
    expect(dm.hasSettledWritesPending()).toBe(true);

    state.config.remoteArray = [
      ['comment:old', 1],
      ['comment:new', 1],
    ];
    await materialize(dm, state);
    expect(dm.hasSettledWritesPending()).toBe(false);
  });

  it('keeps an accepted delete subtracted until membership drops the row', async () => {
    // The mirror case: membership still lists the row until the SSP publishes
    // its removal, so without this the deleted row flashes back.
    const { dm, state } = setup({
      remoteArray: [
        ['comment:old', 1],
        ['comment:new', 1],
      ],
      localArray: [['comment:old', 1]],
    });

    dm.noteWriteSettled('comment:new', 'delete');

    expect(ids(await materialize(dm, state))).toEqual(['comment:old']);
  });

  it('only grants grace to rows the local view still says match', async () => {
    // `localArray` is the local SSP's answer to "does this row match the
    // predicate". A settled write that does NOT match must not be forced into
    // the render set — same rule the pending-write union already follows.
    const { dm, state } = setup({
      remoteArray: [['comment:old', 1]],
      localArray: [['comment:old', 1]],
    });

    dm.noteWriteSettled('comment:new', 'create');

    expect(ids(await materialize(dm, state))).toEqual(['comment:old']);
  });
});
