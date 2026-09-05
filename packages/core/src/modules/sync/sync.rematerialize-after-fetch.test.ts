import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

// A LIVE (or poll) UPDATE for a row already in membership fetches the new body
// into the local store — and that can be the end of it: the circuit sees the
// same id-set and emits no stream update, and nothing else re-reads the query.
// Measured on staging: a `conversation` row reached the store at `_00_rv` 8
// while the query kept rendering version 2 for minutes. `runSyncForQuery`
// must therefore land a synthetic re-materialize whenever a fetch produced
// no stream update of its own.

function makeSync(opts: { streamUpdatePending: boolean }) {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const queryState: any = {
    config: {
      id: new RecordId('_00_query', 'h1'),
      // Membership already knows the newer version; the circuit still holds 1.
      localArray: [['conversation:c1', 1]],
      remoteArray: [['conversation:c1', 8]],
      membershipKnown: true,
    },
  };
  const calls: string[] = [];
  const dataModule: any = {
    getQueryByHash: vi.fn().mockReturnValue(queryState),
    getPendingRecordIds: async () => ({ writes: new Set(), deletes: new Set() }),
    beginFetching: vi.fn(() => calls.push('beginFetching')),
    endFetching: vi.fn(() => calls.push('endFetching')),
    recordRemoteFetch: vi.fn(),
    updateQueryLocalArray: vi.fn(),
    scheduleRematerialize: vi.fn(() => calls.push('scheduleRematerialize')),
    // First flush reports whether a real stream update was pending; a second
    // flush (after the synthetic schedule) always lands something.
    flushPendingStreamUpdate: vi
      .fn()
      .mockImplementationOnce(async () => {
        calls.push('flush');
        return opts.streamUpdatePending;
      })
      .mockImplementation(async () => {
        calls.push('flush');
        return true;
      }),
  };
  const sync = new Sp00kySync({} as any, { query: vi.fn() } as any, {} as any, dataModule, {} as any, logger);
  (sync as any).syncEngine = {
    syncRecords: vi.fn(async () => {
      calls.push('syncRecords');
      return { remoteFetchMs: 3, stillRemoteIds: [] };
    }),
  };
  const run = () =>
    (sync as any).runSyncForQuery('h1', {
      added: [],
      updated: [{ id: 'conversation:c1', version: 8 }],
      removed: [],
    }) as Promise<void>;
  return { run, dataModule, calls };
}

describe('runSyncForQuery re-materializes a fetched row the circuit stayed silent about', () => {
  it('schedules and lands a synthetic re-materialize before going idle', async () => {
    const { run, dataModule, calls } = makeSync({ streamUpdatePending: false });
    await run();
    expect(dataModule.scheduleRematerialize).toHaveBeenCalledWith('h1');
    expect(dataModule.flushPendingStreamUpdate).toHaveBeenCalledTimes(2);
    // The re-render lands BEFORE `endFetching`, so `idle` never races ahead
    // of the rows it describes.
    expect(calls).toEqual([
      'beginFetching',
      'syncRecords',
      'flush',
      'scheduleRematerialize',
      'flush',
      'endFetching',
    ]);
  });

  it('leaves a real stream update alone', async () => {
    const { run, dataModule } = makeSync({ streamUpdatePending: true });
    await run();
    expect(dataModule.scheduleRematerialize).not.toHaveBeenCalled();
    expect(dataModule.flushPendingStreamUpdate).toHaveBeenCalledTimes(1);
  });
});
