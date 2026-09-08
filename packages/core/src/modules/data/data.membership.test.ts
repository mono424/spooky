import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';
import type { QueryPlan } from '@spooky-sync/query-builder';
import type { QueryState, RecordVersionArray } from '../../types';

/**
 * A query's rows are its MEMBERSHIP — the id-set the server put in
 * `_00_list_ref` — not "every locally cached body that matches the WHERE".
 *
 * Those two disagree whenever a row leaves a query's window while still existing
 * upstream: `SyncEngine.handleRemovedRecords` keeps the local body and never
 * re-fetches it, so a predicate re-scan finds that stale body still matching and
 * keeps rendering the row. That is the "removed item comes back" bug, and it was
 * permanent offline because membership used to live only on the session-salted
 * `_00_query` row (wiped on reload) while bodies were durable.
 */

const noop = () => {};

function makeLogger(): any {
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

const schema = { tables: [{ name: 'thread', columns: {} }] } as any;

const plan: QueryPlan = { table: 'thread', where: [['done', '=', false]] } as any;

function makeQueryState(
  hash: string,
  opts: {
    remoteArray?: RecordVersionArray;
    localArray?: RecordVersionArray;
    membershipKnown?: boolean;
    membershipKey?: string;
    withPlan?: boolean;
  } = {}
): QueryState {
  return {
    config: {
      id: new RecordId('_00_query', hash),
      surql: 'SELECT * FROM thread WHERE done = false;',
      plan: opts.withPlan === false ? undefined : plan,
      params: {},
      localArray: opts.localArray ?? [],
      remoteArray: opts.remoteArray ?? [],
      membershipKnown: opts.membershipKnown,
      membershipKey: opts.membershipKey,
      ttl: '10m',
      lastActiveAt: new Date(),
      tableName: 'thread',
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

/**
 * Local store holding three thread bodies, ALL of which match the query's
 * predicate. `select` mimics the real engines: when `plan.ids` is set it returns
 * exactly those ids (the membership path); otherwise it returns everything (the
 * predicate-scan path).
 */
function makeLocal(pendingRows: Array<{ recordId: RecordId; mutationType: string }> = []) {
  const bodies = new Map(
    ['a', 'b', 'c'].map((k) => [`thread:${k}`, { id: new RecordId('thread', k), title: k }])
  );
  const windowRows = new Map<string, { ids: RecordVersionArray; confirmed?: boolean }>();
  const local: any = {
    epoch: 1,
    bodies,
    windowRows,
    select: vi.fn(async (p: QueryPlan) => {
      if (p.ids) {
        return (p.ids as RecordId[])
          .map((id) => bodies.get(id.toString()))
          .filter((r): r is NonNullable<typeof r> => r !== undefined);
      }
      return Array.from(bodies.values());
    }),
    query: vi.fn(async (sql: string, vars?: any) => {
      if (sql.includes('_00_pending_mutations')) return [pendingRows];
      // `createNewQuery` reads the `_00_query` row, then CREATEs it when absent.
      // A reload always takes the absent path: the id is session-salted.
      if (sql.includes('FROM ONLY')) return [null];
      if (sql.startsWith('CREATE')) return [{ id: vars.id, ...vars.data }];
      if (sql.startsWith('UPDATE')) return [null];
      return [Array.from(bodies.values())];
    }),
    getById: vi.fn(async (_table: string, id: RecordId) => windowRows.get(String(id.id)) ?? null),
    upsert: vi.fn(async (_t: string, id: RecordId, data: any) => {
      windowRows.set(String(id.id), data);
    }),
  };
  return local;
}

function setup(
  stateOpts: Parameters<typeof makeQueryState>[1] = {},
  pendingRows: Array<{ recordId: RecordId; mutationType: string }> = []
) {
  const hash = 'h1';
  const local = makeLocal(pendingRows);
  const dm = new DataModule({ saveBatch: async () => {} } as any, local, schema, makeLogger(), 100);
  const state = makeQueryState(hash, stateOpts);
  (dm as any).activeQueries.set(hash, state);
  return { dm, state, local, hash };
}

const ids = (rows: Array<Record<string, any>>) => rows.map((r) => String(r.id));
const materialize = (dm: DataModule<any>, state: QueryState, ssp?: RecordVersionArray) =>
  (dm as any).materializeRecords(state, ssp) as Promise<Record<string, any>[]>;

describe('membership-authoritative rendering', () => {
  it('hides a locally cached body that is absent from membership', async () => {
    // `thread:c` still matches the predicate and its body is still cached, but
    // the server dropped it from the window. It must not render.
    const { dm, state } = setup({
      membershipKnown: true,
      remoteArray: [
        ['thread:a', 1],
        ['thread:b', 1],
      ],
    });

    expect(ids(await materialize(dm, state))).toEqual(['thread:a', 'thread:b']);
  });

  it('renders an empty list when membership is known and empty', async () => {
    const { dm, state } = setup({ membershipKnown: true, remoteArray: [] });
    expect(await materialize(dm, state)).toEqual([]);
  });

  it('falls back to the predicate scan when membership was never established', async () => {
    // A query first run on this device: there is nothing authoritative to render
    // from, so an offline first paint must still show the cached bodies.
    const { dm, state, local } = setup({ membershipKnown: false });
    expect(ids(await materialize(dm, state))).toEqual(['thread:a', 'thread:b', 'thread:c']);
    expect(local.select).toHaveBeenCalledWith(
      expect.objectContaining({ where: plan.where }),
      expect.anything()
    );
  });

  it('drops where/limit/offset and keeps orderBy on the membership path', async () => {
    const { dm, state, local } = setup({ membershipKnown: true, remoteArray: [['thread:a', 1]] });
    state.config.plan = { ...plan, limit: 50, offset: 0, orderBy: [['title', 'asc']] } as any;

    await materialize(dm, state);

    const passed = local.select.mock.calls.at(-1)![0] as QueryPlan;
    expect(passed.where).toBeUndefined();
    expect(passed.limit).toBeUndefined();
    expect(passed.offset).toBeUndefined();
    expect(passed.orderBy).toEqual([['title', 'asc']]);
  });

  describe('optimistic writes', () => {
    it('renders a pending create the server has not acknowledged yet', async () => {
      // Not in remoteArray, but the SSP (fed every local write) says it matches.
      const { dm, state } = setup(
        {
          membershipKnown: true,
          remoteArray: [['thread:a', 1]],
          localArray: [
            ['thread:a', 1],
            ['thread:c', 1],
          ],
        },
        [{ recordId: new RecordId('thread', 'c'), mutationType: 'create' }]
      );

      expect(ids(await materialize(dm, state))).toEqual(['thread:a', 'thread:c']);
    });

    it('hides a pending update that moved a row OUT of the window', async () => {
      // Pending write, but the SSP no longer lists it: the local body stopped
      // matching. Unioning pending ids blindly would wrongly re-admit it.
      const { dm, state } = setup(
        {
          membershipKnown: true,
          remoteArray: [['thread:a', 1]],
          localArray: [['thread:a', 1]],
        },
        [{ recordId: new RecordId('thread', 'c'), mutationType: 'update' }]
      );

      expect(ids(await materialize(dm, state))).toEqual(['thread:a']);
    });

    it('hides a row the server still lists while our DELETE is queued', async () => {
      const { dm, state } = setup(
        {
          membershipKnown: true,
          remoteArray: [
            ['thread:a', 1],
            ['thread:b', 1],
          ],
        },
        [{ recordId: new RecordId('thread', 'b'), mutationType: 'delete' }]
      );

      expect(ids(await materialize(dm, state))).toEqual(['thread:a']);
    });

    it('prefers the fresher sspArray over the persisted localArray', async () => {
      const { dm, state } = setup(
        { membershipKnown: true, remoteArray: [['thread:a', 1]], localArray: [] },
        [{ recordId: new RecordId('thread', 'c'), mutationType: 'create' }]
      );

      const rows = await materialize(dm, state, [
        ['thread:a', 1],
        ['thread:c', 1],
      ]);
      expect(ids(rows)).toEqual(['thread:a', 'thread:c']);
    });
  });

  describe('render order', () => {
    it('sorts an unordered query so both paints agree', async () => {
      // `_00_list_ref` is selected without an ORDER BY, so membership arrives
      // shuffled. The first paint came from the local scan in id order, so
      // rendering server order here is what made lists visibly reorder about a
      // second after load.
      const { dm, hash, state } = setup();
      state.config.plan = { table: 'thread', where: [['done', '=', false]] } as any;

      const ids = await (dm as any).buildRenderIds(state.config, [
        ['thread:c', 1],
        ['thread:a', 1],
        ['thread:b', 1],
      ]);

      expect(ids.map(String)).toEqual(['thread:a', 'thread:b', 'thread:c']);
    });

    it('leaves an explicitly ordered query to the engine', async () => {
      const { dm, state } = setup();
      state.config.plan = { table: 'thread', orderBy: [['created', 'desc']] } as any;

      const ids = await (dm as any).buildRenderIds(state.config, [
        ['thread:c', 1],
        ['thread:a', 1],
      ]);

      // Untouched: the engine applies the ORDER BY over the id set.
      expect(ids.map(String)).toEqual(['thread:c', 'thread:a']);
    });

    it('preserves the slice order of a windowed query', async () => {
      // For a window the id-set order IS the window: sorting it would reorder
      // rows within the page.
      const { dm, state } = setup();
      state.config.surql = 'SELECT * FROM thread LIMIT 50 START 100;';
      state.config.plan = { table: 'thread' } as any;

      const ids = await (dm as any).buildRenderIds(state.config, [
        ['thread:c', 1],
        ['thread:a', 1],
      ]);

      expect(ids.map(String)).toEqual(['thread:c', 'thread:a']);
    });
  });

  describe('durability across a reload', () => {
    it('updateQueryRemoteArray latches membership and writes _00_window', async () => {
      const { dm, state, local, hash } = setup({ membershipKey: 'stable-key' });

      await dm.updateQueryRemoteArray(hash, [['thread:a', 1]]);

      expect(state.config.membershipKnown).toBe(true);
      expect(local.windowRows.get('stable-key')).toMatchObject({ ids: [['thread:a', 1]] });
    });

    it('seeds membership from _00_window under a different session salt', async () => {
      // The reload case: `_00_query` is keyed by a `session::id()`-salted hash, so
      // a reload always lands on a fresh row with empty arrays. Only the durable
      // row, keyed session-independently, can carry the removal across.
      const { dm, local } = setup({ membershipKey: 'stable-key' });
      local.windowRows.set('stable-key', { ids: [['thread:a', 1]] });

      const fresh = await (dm as any).createNewQuery({
        recordId: new RecordId('_00_query', 'a-totally-different-salted-hash'),
        surql: 'SELECT * FROM thread WHERE done = false;',
        params: {},
        ttl: '10m',
        tableName: 'thread',
        plan,
        membershipKey: 'stable-key',
      });

      expect(fresh.config.membershipKnown).toBe(true);
      expect(fresh.config.remoteArray).toEqual([['thread:a', 1]]);
      // `thread:b`/`thread:c` bodies are still cached and still match, but are
      // not in the persisted list — the reported bug, offline.
      expect(ids(fresh.records)).toEqual(['thread:a']);
    });

    it('stays scan-based when no durable row exists', async () => {
      const { dm } = setup({ membershipKey: 'never-written' });

      const fresh = await (dm as any).createNewQuery({
        recordId: new RecordId('_00_query', 'h-cold'),
        surql: 'SELECT * FROM thread WHERE done = false;',
        params: {},
        ttl: '10m',
        tableName: 'thread',
        plan,
        membershipKey: 'never-written',
      });

      expect(fresh.config.membershipKnown).toBeFalsy();
      expect(ids(fresh.records)).toEqual(['thread:a', 'thread:b', 'thread:c']);
    });

    // The `_00_query` row as read alongside the edges. `present: true` with
    // `state: null` is a server that predates the marker.
    const meta = (
      over: Partial<{
        present: boolean;
        rowCount: number | null;
        state: 'materializing' | 'ready' | null;
      }> = {}
    ) => ({ present: true, rowCount: null, state: null, ...over });

    it('ignores an empty id-set while the server is still publishing', async () => {
      // The registration race: the SSP writes `state = materializing`, hands
      // the view's initial edges to a coalescing flusher and answers
      // `fn::query::register` before they land, so the read that follows says
      // nothing about the query being empty. Believing it rendered a blank
      // list AND persisted the blankness.
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      const outcome = await dm.updateQueryRemoteArray(hash, [], {
        meta: meta({ rowCount: 26, state: 'materializing' }),
      });

      expect(outcome).toBe('ignored');
      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);
      expect(local.windowRows.has('stable-key')).toBe(false);
      expect(state.serverState).toBe('materializing');
    });

    it('ignores an empty id-set while the server still reports rows', async () => {
      // `ready` but `rowCount 26` with no edges: the row and the edges
      // disagree (a stranded view, metrics lag). No number of such reads may
      // be taken as "empty".
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      for (let i = 0; i < 5; i++) {
        expect(
          await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: 26, state: 'ready' }) })
        ).toBe('ignored');
      }

      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);
      expect(local.windowRows.has('stable-key')).toBe(false);
    });

    it('believes an empty id-set the moment the server says ready with zero rows', async () => {
      // The genuine transition — the last row left the window. The SSP flips
      // `ready` inside the transaction that writes the edges, so this cannot
      // be observed mid-publish. Honouring it is what stops a removed row
      // resurrecting from the local body cache.
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      const outcome = await dm.updateQueryRemoteArray(hash, [], {
        meta: meta({ rowCount: 0, state: 'ready' }),
      });

      expect(outcome).toBe('applied');
      expect(state.config.membershipKnown).toBe(true);
      expect(state.config.remoteArray).toEqual([]);
      expect(local.windowRows.get('stable-key')).toEqual(
        expect.objectContaining({ ids: [], confirmed: true })
      );
    });

    it('believes ready + zero rows with nothing held yet, too', async () => {
      // A genuinely empty query on a cold device must not sit on a spinner
      // waiting for rows that are never coming.
      const { dm, state, hash } = setup({ membershipKey: 'stable-key' });

      const outcome = await dm.updateQueryRemoteArray(hash, [], {
        meta: meta({ rowCount: 0, state: 'ready' }),
      });

      expect(outcome).toBe('applied');
      expect(state.config.membershipKnown).toBe(true);
    });

    it('treats a missing _00_query row as a lost view, never as an empty one', async () => {
      // A TTL sweep, an SSP reset or a scheduler wipe took the row (and, with
      // it, every edge). The client keeps what it renders, says so once, and
      // asks for a re-registration. Believing the empty here is what blanked
      // lists after every sweep and kept them blank across reloads.
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });
      const onViewLost = vi.fn();
      dm.onViewLost = onViewLost;

      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ present: false }) })).toBe(
        'view-lost'
      );
      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ present: false }) })).toBe(
        'view-lost'
      );

      expect(onViewLost).toHaveBeenCalledTimes(1);
      expect(onViewLost).toHaveBeenCalledWith(hash);
      expect(state.viewLost).toBe(true);
      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);
      expect(state.config.membershipKnown).toBe(true);
      expect(local.windowRows.has('stable-key')).toBe(false);
    });

    it('clears the lost-view mark once a readable row is back', async () => {
      const { dm, state, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });
      dm.onViewLost = vi.fn();
      await dm.updateQueryRemoteArray(hash, [], { meta: meta({ present: false }) });
      expect(state.viewLost).toBe(true);

      // The re-registration republished the view.
      const outcome = await dm.updateQueryRemoteArray(
        hash,
        [
          ['thread:a', 1],
          ['thread:b', 1],
        ],
        { meta: meta({ rowCount: 2, state: 'ready' }) }
      );

      expect(outcome).toBe('applied');
      expect(state.viewLost).toBe(false);
      expect(state.serverState).toBe('ready');
    });

    it('stays non-authoritative when nothing is held and the row is missing', async () => {
      // A cold query whose row is not readable yet: nothing to lose, nothing
      // to believe. The register lifecycle retries; no re-registration storm.
      const { dm, state, hash } = setup({ membershipKey: 'stable-key' });
      const onViewLost = vi.fn();
      dm.onViewLost = onViewLost;

      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ present: false }) })).toBe(
        'ignored'
      );

      expect(onViewLost).not.toHaveBeenCalled();
      expect(state.config.membershipKnown).toBeFalsy();
      expect(state.viewLost).toBeFalsy();
    });

    it('treats an empty set that comes with no row information as a lost view', async () => {
      const { dm, state, hash } = setup({
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });
      dm.onViewLost = vi.fn();
      expect(await dm.updateQueryRemoteArray(hash, [])).toBe('view-lost');
      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);
    });

    it('judges a server that predates `state` on rowCount alone', async () => {
      const { dm, state, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: 3 }) })).toBe(
        'ignored'
      );
      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: null }) })).toBe(
        'ignored'
      );
      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);

      expect(await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: 0 }) })).toBe(
        'applied'
      );
      expect(state.config.remoteArray).toEqual([]);
    });

    it('never grows to believe an unexplained empty on repeated reads', async () => {
      // There is no retry budget any more: a server that cannot explain an
      // empty must not be able to erase a device's data by repeating itself.
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      for (let i = 0; i < 6; i++) {
        await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: null, state: 'ready' }) });
      }

      expect(state.config.remoteArray).toEqual([['thread:a', 1]]);
      expect(local.windowRows.has('stable-key')).toBe(false);
    });

    it('applies an empty set the caller verified upstream', async () => {
      // The LIVE removal path checked that every held id is gone from the
      // server before asking: that is a real answer even without a row.
      const { dm, state, local, hash } = setup({
        membershipKey: 'stable-key',
        remoteArray: [['thread:a', 1]],
        membershipKnown: true,
      });

      const outcome = await dm.updateQueryRemoteArray(hash, [], { verifiedRemoval: true });

      expect(outcome).toBe('applied');
      expect(state.config.remoteArray).toEqual([]);
      expect(local.windowRows.get('stable-key')).toEqual(
        expect.objectContaining({ ids: [], confirmed: true })
      );
    });

    it('marks every applied set as confirmed', async () => {
      // Every set that lands is server-backed now, so the durable mirror may
      // always be trusted on the next boot, empty or not.
      const { dm, local, hash } = setup({ membershipKey: 'stable-key' });

      await dm.updateQueryRemoteArray(hash, [['thread:a', 1]]);
      expect(local.windowRows.get('stable-key')).toMatchObject({ confirmed: true });

      await dm.updateQueryRemoteArray(hash, [], { meta: meta({ rowCount: 0, state: 'ready' }) });
      expect(local.windowRows.get('stable-key')).toEqual(
        expect.objectContaining({ ids: [], confirmed: true })
      );
    });

    it('seeds a known-empty membership from a confirmed empty durable row', async () => {
      // The reload after a server-confirmed empty: the list must stay empty
      // instead of re-admitting every cached body until the next poll blanks it.
      const { dm, local } = setup({ membershipKey: 'stable-key' });
      local.windowRows.set('stable-key', { ids: [], confirmed: true });

      const fresh = await (dm as any).createNewQuery({
        recordId: new RecordId('_00_query', 'h-confirmed-empty'),
        surql: 'SELECT * FROM thread WHERE done = false;',
        params: {},
        ttl: '10m',
        tableName: 'thread',
        plan,
        membershipKey: 'stable-key',
      });

      expect(fresh.config.membershipKnown).toBe(true);
      expect(fresh.config.remoteArray).toEqual([]);
      expect(fresh.records).toEqual([]);
    });

    it('does not seed membership from an empty durable row', async () => {
      // Self-heals devices poisoned before the guard existed: an empty durable
      // row is indistinguishable from "never had membership", so it must fall
      // back to the scan rather than paint an empty list.
      const { dm, local } = setup({ membershipKey: 'stable-key' });
      local.windowRows.set('stable-key', { ids: [] });

      const fresh = await (dm as any).createNewQuery({
        recordId: new RecordId('_00_query', 'h-poisoned'),
        surql: 'SELECT * FROM thread WHERE done = false;',
        params: {},
        ttl: '10m',
        tableName: 'thread',
        plan,
        membershipKey: 'stable-key',
      });

      expect(fresh.config.membershipKnown).toBeFalsy();
      expect(ids(fresh.records)).toEqual(['thread:a', 'thread:b', 'thread:c']);
    });

    it('derives the membership key without the session salt', async () => {
      const { dm } = setup();
      const key = () => (dm as any).calculateMembershipKey({ surql: 'X', params: {} });

      const before = await key();
      dm.setSessionId('session-two');
      expect(await key()).toBe(before);

      // ...whereas the `_00_query` hash intentionally moves with the session.
      const salted = await (dm as any).calculateHash({ surql: 'X', params: {} });
      dm.setSessionId('session-three');
      expect(await (dm as any).calculateHash({ surql: 'X', params: {} })).not.toBe(salted);
    });
  });
});
