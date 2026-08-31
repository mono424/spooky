import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';

/**
 * `getPendingRecordIds` used to run a full `SELECT ... FROM
 * _00_pending_mutations` on EVERY materialization of EVERY query — a round trip
 * down the local engine's single-flight op queue, paid tens of times per ingest
 * against an outbox that is usually empty. It is now cached, invalidated when
 * the outbox actually changes, with a short TTL as a backstop.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

/** DataModule over a local store that counts outbox reads and can be told what
 *  to return. */
function makeModule(rows: { recordId: RecordId<string>; mutationType: string }[] = []) {
  const state = { rows, reads: 0 };
  const local = {
    query: vi.fn(async () => {
      state.reads++;
      return [state.rows];
    }),
  };
  const dm = new DataModule(
    {} as any,
    local as any,
    { tables: [] } as any,
    makeLogger(),
    100
  );
  return { dm, state };
}

const pending = (id: string, mutationType = 'update') => ({
  recordId: new RecordId('game', id),
  mutationType,
});

describe('DataModule pending-id cache', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('reads the outbox once for repeated calls', async () => {
    const { dm, state } = makeModule([pending('a')]);
    await dm.getPendingRecordIds();
    await dm.getPendingRecordIds();
    await dm.getPendingRecordIds();
    expect(state.reads).toBe(1);
  });

  it('collapses a burst of CONCURRENT calls into one read', async () => {
    // The real shape: one ingest fans out to many queries materializing at once.
    const { dm, state } = makeModule([pending('a')]);
    await Promise.all([
      dm.getPendingRecordIds(),
      dm.getPendingRecordIds(),
      dm.getPendingRecordIds(),
      dm.getPendingRecordIds(),
    ]);
    expect(state.reads).toBe(1);
  });

  it('hands out COPIES, so a caller mutating the sets cannot poison the cache', async () => {
    // buildRenderIds merges the settled-write ids into what it gets back.
    const { dm } = makeModule([pending('a')]);
    const first = await dm.getPendingRecordIds();
    first.writes.add('game:injected');
    const second = await dm.getPendingRecordIds();
    expect(second.writes.has('game:injected')).toBe(false);
    expect([...second.writes]).toEqual(['game:a']);
  });

  it('re-reads after a mutation settles (its outbox row is gone)', async () => {
    const { dm, state } = makeModule([pending('a')]);
    await dm.getPendingRecordIds();
    expect(state.reads).toBe(1);

    state.rows = [];
    dm.noteWriteSettled('game:a', 'update');
    const after = await dm.getPendingRecordIds();

    expect(state.reads).toBe(2);
    expect(after.writes.size).toBe(0);
  });

  it('re-reads once the TTL backstop lapses', async () => {
    // Covers any path that removes an outbox row without telling us: staleness
    // is bounded to a tick rather than lasting the session.
    const { dm, state } = makeModule([pending('a')]);
    await dm.getPendingRecordIds();
    expect(state.reads).toBe(1);

    vi.setSystemTime(Date.now() + 1_000);
    await dm.getPendingRecordIds();
    expect(state.reads).toBe(2);
  });

  it('does NOT cache a failed read', async () => {
    // Empty sets are this call's fallback, not a claim that the outbox is empty.
    const { dm, state } = makeModule([pending('a')]);
    (dm as any).local.query = vi.fn(async () => {
      state.reads++;
      throw new Error('engine down');
    });
    const failed = await dm.getPendingRecordIds();
    expect(failed.writes.size).toBe(0);
    expect(state.reads).toBe(1);

    await dm.getPendingRecordIds();
    expect(state.reads).toBe(2);
  });

  it('splits writes from deletes', async () => {
    const { dm } = makeModule([pending('a'), pending('b', 'delete')]);
    const { writes, deletes } = await dm.getPendingRecordIds();
    expect([...writes]).toEqual(['game:a']);
    expect([...deletes]).toEqual(['game:b']);
  });
});
