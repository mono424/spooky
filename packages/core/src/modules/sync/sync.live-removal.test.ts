import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

// A LIVE `_00_list_ref` DELETE is how one window learns another deleted a record.
//
// `remoteArray` — the authoritative membership rows are now rendered FROM — used
// to be written only by registration and the poll, so a LIVE removal left the
// departed id in the list (in memory AND persisted) until the next poll tick: up
// to 5s of showing a deleted row, and on a page that then went offline, forever.
//
// A removal-only diff also gets no re-render for free: `runSyncForQuery` leaves
// `fetching` false, and a removal needs no record fetch to trigger a stream
// update, so the notify has to be forced.

function makeSync(opts: { membershipKnown?: boolean } = {}) {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const queryId = new RecordId('_00_query', 'h1');
  const queryState: any = {
    config: {
      id: queryId,
      localArray: [
        ['thread:a', 1],
        ['thread:b', 1],
      ],
      remoteArray: [
        ['thread:a', 1],
        ['thread:b', 1],
      ],
      membershipKnown: opts.membershipKnown ?? true,
      membershipKey: 'stable-key',
    },
  };

  const updateQueryRemoteArray = vi.fn(async (_h: string, next: any) => {
    queryState.config.remoteArray = next;
  });
  const notifyQuerySynced = vi.fn().mockResolvedValue(undefined);
  const dataModule: any = {
    getQueryById: vi.fn().mockReturnValue(queryState),
    getQueryByHash: vi.fn().mockReturnValue(queryState),
    updateQueryRemoteArray,
    notifyQuerySynced,
    getActiveQueryHashes: () => ['h1'],
    getPendingRecordIds: async () => ({ writes: new Set(), deletes: new Set() }),
  };

  const sync = new Sp00kySync(
    {} as any,
    { query: vi.fn() } as any,
    {} as any,
    dataModule,
    {} as any,
    logger
  );
  // `runSyncForQuery` drives the engine + scheduler; both are out of scope here.
  (sync as any).runSyncForQuery = vi.fn().mockResolvedValue(undefined);

  const live = (action: 'CREATE' | 'UPDATE' | 'DELETE', id: string, version: number) =>
    (sync as any).handleRemoteListRefChange(
      action,
      queryId,
      new RecordId('thread', id),
      version
    ) as Promise<void>;

  return { sync, queryState, updateQueryRemoteArray, notifyQuerySynced, live };
}

describe('LIVE list_ref removal → membership', () => {
  beforeEach(() => vi.clearAllMocks());

  it('drops the removed id from remoteArray immediately', async () => {
    const { queryState, updateQueryRemoteArray, live } = makeSync();

    await live('DELETE', 'b', 2);

    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [['thread:a', 1]]);
    expect(queryState.config.remoteArray).toEqual([['thread:a', 1]]);
  });

  it('forces a re-render for a removal-only diff', async () => {
    const { notifyQuerySynced, live } = makeSync();
    await live('DELETE', 'b', 2);
    expect(notifyQuerySynced).toHaveBeenCalledWith('h1');
  });

  it('does not force a re-render when rows were added (the stream update covers it)', async () => {
    const { notifyQuerySynced, live } = makeSync();
    await live('CREATE', 'c', 1);
    expect(notifyQuerySynced).not.toHaveBeenCalled();
  });

  it('adds a newly-arrived id to membership', async () => {
    const { updateQueryRemoteArray, live } = makeSync();

    await live('CREATE', 'c', 1);

    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [
      ['thread:a', 1],
      ['thread:b', 1],
      ['thread:c', 1],
    ]);
  });

  // Every tab that ingested a write optimistically (its own, or one relayed
  // from another tab) already holds the row at the server's version, so the
  // fetch diff is empty. Membership still has to be recorded, or the row lives
  // on the settled-write grace alone until the poll catches it.
  it('adds membership for a row the circuit already holds at the server version', async () => {
    const { queryState, updateQueryRemoteArray, sync, live } = makeSync();
    queryState.config.localArray = [
      ['thread:a', 1],
      ['thread:b', 1],
      ['thread:c', 1],
    ];

    await live('CREATE', 'c', 1);

    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [
      ['thread:a', 1],
      ['thread:b', 1],
      ['thread:c', 1],
    ]);
    // …without a refetch: the diff handed to the sync is empty.
    expect((sync as any).runSyncForQuery).toHaveBeenCalledWith('h1', {
      added: [],
      updated: [],
      removed: [],
    });
  });

  it('records a bumped version on UPDATE without rewriting an unchanged list', async () => {
    const { updateQueryRemoteArray, live } = makeSync();

    await live('UPDATE', 'b', 1);
    expect(updateQueryRemoteArray).not.toHaveBeenCalled();

    await live('UPDATE', 'b', 2);
    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [
      ['thread:a', 1],
      ['thread:b', 2],
    ]);
  });

  it('leaves membership alone while it is still unknown', async () => {
    // Nothing authoritative has arrived yet, so there is no list to amend —
    // registration will supply the whole thing shortly.
    const { updateQueryRemoteArray, live } = makeSync({ membershipKnown: false });
    await live('DELETE', 'b', 2);
    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
  });

  it('is a no-op for an unknown query', async () => {
    const { sync, updateQueryRemoteArray } = makeSync();
    (sync as any).dataModule.getQueryById = vi.fn().mockReturnValue(undefined);

    await (sync as any).handleRemoteListRefChange(
      'DELETE',
      new RecordId('_00_query', 'other'),
      new RecordId('thread', 'b'),
      2
    );

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
  });
});
