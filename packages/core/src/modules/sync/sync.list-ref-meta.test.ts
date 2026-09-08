import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

/**
 * The list_ref readers (registration, poll, targeted re-read) read the
 * `_00_query` row's `rowCount` and publish `state` next to the edges, dedupe
 * duplicate edges, and hand the DataModule's verdict back to the poll so a
 * rejected snapshot (SSP still publishing, view lost) leaves the query alone.
 */

function makeSync(states: Record<string, { held: Array<[string, number]> }>) {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const remote: any = { query: vi.fn() };
  const queryStates: Record<string, any> = {};
  for (const [hash, { held }] of Object.entries(states)) {
    queryStates[hash] = {
      config: {
        id: new RecordId('_00_query', hash),
        localArray: [],
        remoteArray: held,
        subqueryRemoteArray: undefined,
        membershipKnown: held.length > 0,
      },
    };
  }
  const updateQueryRemoteArray = vi.fn().mockResolvedValue('applied');
  const dataModule: any = {
    getCurrentUserId: () => null,
    getQueryByHash: (h: string) => queryStates[h],
    getQueryById: (id: RecordId) => queryStates[String(id.id)],
    getActiveQueryHashes: () => Object.keys(queryStates),
    updateQueryRemoteArray,
    scheduleRematerialize: vi.fn(),
    notifyQuerySynced: vi.fn().mockResolvedValue(undefined),
  };
  const sync = new Sp00kySync({} as any, remote, {} as any, dataModule, {} as any, logger);
  (sync as any).syncQuery = vi.fn().mockResolvedValue(undefined);
  (sync as any).applySubqueryChildren = vi.fn().mockResolvedValue(undefined);
  return { sync, remote, dataModule, queryStates, updateQueryRemoteArray };
}

const rid = (s: string) => {
  const [table, id] = s.split(':');
  return new RecordId(table, id);
};

describe('list_ref snapshot meta', () => {
  beforeEach(() => vi.clearAllMocks());

  it('reads the row state next to the edges and dedupes duplicate edges (single form)', async () => {
    const { sync, remote } = makeSync({ h1: { held: [] } });
    remote.query.mockResolvedValue([
      [
        { out: rid('thread:a'), version: 1 },
        { out: rid('thread:a'), version: 2 },
        { out: rid('thread:b'), version: 1 },
      ],
      { rowCount: 2, state: 'ready' },
      [],
    ]);

    const snapshot = await (sync as any).fetchListRefSnapshot(rid('_00_query:h1'));

    expect(snapshot.primary.sort()).toEqual([
      ['thread:a', 2],
      ['thread:b', 1],
    ]);
    expect(snapshot.meta).toEqual({ present: true, rowCount: 2, state: 'ready' });
    expect(snapshot.rowCount).toBe(2);
  });

  it('reports a missing row as present: false', async () => {
    const { sync, remote } = makeSync({ h1: { held: [] } });
    remote.query.mockResolvedValue([[], null, []]);
    const snapshot = await (sync as any).fetchListRefSnapshot(rid('_00_query:h1'));
    expect(snapshot.meta).toEqual({ present: false, rowCount: null, state: null });
  });

  it('treats an unknown state value as no state (legacy server)', async () => {
    const { sync, remote } = makeSync({ h1: { held: [] } });
    remote.query.mockResolvedValue([[], { rowCount: 0 }, []]);
    const snapshot = await (sync as any).fetchListRefSnapshot(rid('_00_query:h1'));
    expect(snapshot.meta).toEqual({ present: true, rowCount: 0, state: null });
  });

  it('fills meta per query in the batched form and dedupes', async () => {
    const { sync, remote } = makeSync({ h1: { held: [] }, h2: { held: [] } });
    remote.query.mockResolvedValue([
      [
        { in: rid('_00_query:h1'), out: rid('thread:a'), version: 1, parent: null },
        { in: rid('_00_query:h1'), out: rid('thread:a'), version: 3, parent: null },
        { in: rid('_00_query:h2'), out: rid('thread:z'), version: 1, parent: null },
      ],
      [
        { id: rid('_00_query:h1'), rowCount: 1, state: 'materializing' },
        { id: rid('_00_query:h2'), rowCount: 1, state: 'ready' },
      ],
    ]);

    const out = await (sync as any).fetchListRefSnapshots(['h1', 'h2']);

    expect(out.get('h1').primary).toEqual([['thread:a', 3]]);
    expect(out.get('h1').meta).toEqual({ present: true, rowCount: 1, state: 'materializing' });
    expect(out.get('h2').meta).toEqual({ present: true, rowCount: 1, state: 'ready' });
  });

  it('re-reads a held query one at a time when its row is missing from the batch', async () => {
    const { sync, remote } = makeSync({ h1: { held: [['thread:a', 1]] }, h2: { held: [] } });
    // Batch: no edges and no row for h1, which we hold rows for.
    remote.query.mockResolvedValue([[], [{ id: rid('_00_query:h2'), rowCount: 0, state: 'ready' }]]);
    const single = vi.fn().mockResolvedValue({
      primary: [],
      subquery: [],
      rowCount: null,
      meta: { present: false, rowCount: null, state: null },
    });
    (sync as any).fetchListRefSnapshot = single;

    const out = await (sync as any).fetchListRefSnapshots(['h1', 'h2']);

    expect(single).toHaveBeenCalled();
    expect(out.get('h1').meta.present).toBe(false);
  });

  describe('applyListRefSnapshot', () => {
    const snapshot = (primary: Array<[string, number]>, meta: any) => ({
      primary,
      subquery: [],
      rowCount: meta.rowCount,
      meta,
    });

    it('hands the verdict back and touches nothing when the set is rejected', async () => {
      const { sync, dataModule, updateQueryRemoteArray } = makeSync({ h1: { held: [['thread:a', 1]] } });
      updateQueryRemoteArray.mockResolvedValue('ignored');

      const result = await (sync as any).applyListRefSnapshot(
        'h1',
        snapshot([], { present: true, rowCount: 1, state: 'materializing' })
      );

      expect(result).toEqual({ changed: false, outcome: 'ignored' });
      expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [], {
        meta: { present: true, rowCount: 1, state: 'materializing' },
      });
      expect(dataModule.scheduleRematerialize).not.toHaveBeenCalled();
      expect(dataModule.notifyQuerySynced).not.toHaveBeenCalled();
      expect((sync as any).applySubqueryChildren).not.toHaveBeenCalled();
    });

    it('does the same for a lost view', async () => {
      const { sync, dataModule, updateQueryRemoteArray } = makeSync({ h1: { held: [['thread:a', 1]] } });
      updateQueryRemoteArray.mockResolvedValue('view-lost');
      const result = await (sync as any).applyListRefSnapshot(
        'h1',
        snapshot([], { present: false, rowCount: null, state: null })
      );
      expect(result).toEqual({ changed: false, outcome: 'view-lost' });
      expect(dataModule.notifyQuerySynced).not.toHaveBeenCalled();
    });

    it('re-materializes and re-renders removals when the set is applied', async () => {
      const { sync, dataModule } = makeSync({ h1: { held: [['thread:a', 1], ['thread:b', 1]] } });
      const result = await (sync as any).applyListRefSnapshot(
        'h1',
        snapshot([['thread:a', 1]], { present: true, rowCount: 1, state: 'ready' })
      );
      expect(result).toEqual({ changed: true, outcome: 'applied' });
      expect(dataModule.scheduleRematerialize).toHaveBeenCalledWith('h1');
      expect(dataModule.notifyQuerySynced).toHaveBeenCalledWith('h1');
      expect((sync as any).applySubqueryChildren).toHaveBeenCalled();
    });

    it('is quiet when the server set equals what is held', async () => {
      const { sync, dataModule, updateQueryRemoteArray } = makeSync({ h1: { held: [['thread:a', 1]] } });
      const result = await (sync as any).applyListRefSnapshot(
        'h1',
        snapshot([['thread:a', 1]], { present: true, rowCount: 1, state: 'ready' })
      );
      expect(result).toEqual({ changed: false, outcome: 'unchanged' });
      expect(updateQueryRemoteArray).not.toHaveBeenCalled();
      expect(dataModule.scheduleRematerialize).not.toHaveBeenCalled();
    });
  });
});
