import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

// A LIVE `_00_list_ref` DELETE is how one window learns another deleted a record.
//
// It is also how a view teardown arrives: the TTL sweep, an SSP reset, a
// scheduler wipe and a full republish all delete a view's edges, one DELETE per
// row, indistinguishable from a peer deleting rows. Obeying them on arrival
// blanked painted lists n-1 rows at a time. So removals are held briefly,
// coalesced per query, and VERIFIED upstream before membership moves: an id
// that no longer exists is dropped (its body is deleted inside the same sync
// round); anything else is settled by re-reading the server's whole set for the
// query, where a missing `_00_query` row means "view lost, keep everything".
//
// CREATE/UPDATE are applied on arrival as before.

const COALESCE_MS = 100;

function makeSync(
  opts: {
    membershipKnown?: boolean;
    verify?: { confirmedRemovedIds?: string[]; stillRemoteIds?: string[] };
  } = {}
) {
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
    return 'applied';
  });
  const notifyQuerySynced = vi.fn().mockResolvedValue(undefined);
  const dataModule: any = {
    scheduleRematerialize: vi.fn(),
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
  // It reports which removed ids were verified gone upstream and which still
  // exist there.
  const runSyncForQuery = vi.fn().mockResolvedValue({
    confirmedRemovedIds: opts.verify?.confirmedRemovedIds ?? [],
    stillRemoteIds: opts.verify?.stillRemoteIds ?? [],
  });
  (sync as any).runSyncForQuery = runSyncForQuery;
  // The targeted server re-read a non-confirmed removal falls back to.
  const refetch = vi.fn().mockResolvedValue({ changed: false, outcome: 'unchanged' });
  (sync as any).refetchListRefForQuery = refetch;

  const live = (action: 'CREATE' | 'UPDATE' | 'DELETE', id: string, version: number) =>
    (sync as any).handleRemoteListRefChange(
      action,
      queryId,
      new RecordId('thread', id),
      version
    ) as Promise<void>;
  // Let the coalescing window and the zero-delay re-read timer fire.
  const settle = () => vi.advanceTimersByTimeAsync(COALESCE_MS + 10);

  return {
    sync,
    queryState,
    updateQueryRemoteArray,
    notifyQuerySynced,
    runSyncForQuery,
    refetch,
    live,
    settle,
  };
}

describe('LIVE list_ref removal → membership', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });
  afterEach(() => vi.useRealTimers());

  it('does not move membership on arrival', async () => {
    const { queryState, updateQueryRemoteArray, runSyncForQuery, live } = makeSync({
      verify: { confirmedRemovedIds: ['thread:b'] },
    });

    await live('DELETE', 'b', 2);

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
    expect(runSyncForQuery).not.toHaveBeenCalled();
    expect(queryState.config.remoteArray).toEqual([
      ['thread:a', 1],
      ['thread:b', 1],
    ]);
  });

  it('drops a removed id once it is verified gone upstream', async () => {
    const { queryState, updateQueryRemoteArray, notifyQuerySynced, runSyncForQuery, refetch, live, settle } =
      makeSync({ verify: { confirmedRemovedIds: ['thread:b'] } });

    await live('DELETE', 'b', 2);
    await settle();

    expect(runSyncForQuery).toHaveBeenCalledTimes(1);
    const [, diff] = runSyncForQuery.mock.calls[0];
    expect(diff.added).toEqual([]);
    expect(diff.updated).toEqual([]);
    expect(diff.removed.map(String)).toEqual(['thread:b']);
    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [['thread:a', 1]], {
      verifiedRemoval: true,
    });
    expect(queryState.config.remoteArray).toEqual([['thread:a', 1]]);
    // A removal-only diff gets no re-render for free: it has to be forced.
    expect(notifyQuerySynced).toHaveBeenCalledWith('h1');
    expect(refetch).not.toHaveBeenCalled();
  });

  it('keeps a row that still exists upstream and re-reads the server set instead', async () => {
    // The row left this view's edges but is still there: a membership change
    // to be read back as a whole set, or a view teardown to be recognised by
    // its missing `_00_query` row. Either way, not a blind removal.
    const { queryState, updateQueryRemoteArray, notifyQuerySynced, refetch, live, settle } = makeSync({
      verify: { stillRemoteIds: ['thread:b'] },
    });

    await live('DELETE', 'b', 2);
    await settle();

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
    expect(notifyQuerySynced).not.toHaveBeenCalled();
    expect(queryState.config.remoteArray).toEqual([
      ['thread:a', 1],
      ['thread:b', 1],
    ]);
    expect(refetch).toHaveBeenCalledWith('h1');
  });

  it('coalesces a burst of removals into one verification', async () => {
    // A sweep or a republish delivers a view's whole set within a few ms.
    const { runSyncForQuery, live, settle } = makeSync({
      verify: { confirmedRemovedIds: ['thread:a', 'thread:b'] },
    });

    await live('DELETE', 'a', 2);
    await vi.advanceTimersByTimeAsync(30);
    await live('DELETE', 'b', 2);
    await live('DELETE', 'b', 2);
    await settle();

    expect(runSyncForQuery).toHaveBeenCalledTimes(1);
    const [, diff] = runSyncForQuery.mock.calls[0];
    expect(diff.removed.map(String).sort()).toEqual(['thread:a', 'thread:b']);
  });

  it('falls back to a server re-read when the verification could not run', async () => {
    // `handleRemovedRecords` answers with nothing on a failed existence check:
    // neither confirmed nor still-remote. Nothing may be dropped on that.
    const { updateQueryRemoteArray, refetch, live, settle } = makeSync({ verify: {} });

    await live('DELETE', 'b', 2);
    await settle();

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
    expect(refetch).toHaveBeenCalledWith('h1');
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
    const { queryState, updateQueryRemoteArray, runSyncForQuery, live } = makeSync();
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
    expect(runSyncForQuery).toHaveBeenCalledWith('h1', {
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
    // registration will supply the whole thing shortly. The body delete of a
    // verified-gone row still happens inside the sync round.
    const { updateQueryRemoteArray, runSyncForQuery, live, settle } = makeSync({
      membershipKnown: false,
      verify: { confirmedRemovedIds: ['thread:b'] },
    });

    await live('DELETE', 'b', 2);
    await settle();

    expect(runSyncForQuery).toHaveBeenCalledTimes(1);
    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
  });

  it('is a no-op for an unknown query', async () => {
    const { sync, updateQueryRemoteArray, runSyncForQuery, settle } = makeSync();
    (sync as any).dataModule.getQueryById = vi.fn().mockReturnValue(undefined);

    await (sync as any).handleRemoteListRefChange(
      'DELETE',
      new RecordId('_00_query', 'other'),
      new RecordId('thread', 'b'),
      2
    );
    await settle();

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
    expect(runSyncForQuery).not.toHaveBeenCalled();
  });
});
