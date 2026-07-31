import { describe, it, expect, vi } from 'vitest';
import { UpQueue } from './queue-up';
import { translateSurql } from '../../../services/database/surql-translate';
import { surql, classifySyncError } from '../../../utils/index';

function makeLogger(): any {
  const noop = () => {};
  const l: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  l.child = () => l;
  return l;
}

const ROW = {
  id: '_00_pending_mutations:0001753875000000_0001_tabB',
  mutationType: 'create',
  recordId: 'game:g1',
  data: { title: 'x' },
};

// Shared-tabs: followers commit outbox rows into the shared store and notify
// the leader by id. The notify may be REPLAYED (failover buffering), so the
// enqueue must be idempotent, and only the leader ever drains.
describe('UpQueue.enqueueFromDatabase', () => {
  function makeQueue(rows: Record<string, unknown[]>) {
    const query = vi.fn(async (_sql: string, vars?: Record<string, unknown>) => {
      const ids = (vars?.mutation_ids as unknown[]) ?? [];
      const id = String(ids[0] ?? '');
      return [rows[id] ?? []];
    });
    const local: any = { query };
    return { queue: new UpQueue(local, makeLogger()), query };
  }

  it('loads exactly the forwarded row and enqueues it once', async () => {
    const { queue } = makeQueue({ [ROW.id]: [ROW] });
    await queue.enqueueFromDatabase(ROW.id);
    expect(queue.size).toBe(1);
  });

  it('is idempotent: a replayed notify does not double-enqueue', async () => {
    const { queue, query } = makeQueue({ [ROW.id]: [ROW] });
    await queue.enqueueFromDatabase(ROW.id);
    await queue.enqueueFromDatabase(ROW.id);
    expect(queue.size).toBe(1);
    // The second call short-circuits before even touching the store.
    expect(query).toHaveBeenCalledTimes(1);
  });

  it('a row that never committed is a silent no-op (the app promise already rejected)', async () => {
    const { queue } = makeQueue({});
    await queue.enqueueFromDatabase(ROW.id);
    expect(queue.size).toBe(0);
  });

  // The mock above replaces `local.query` wholesale, so on its own it proves
  // nothing about the SQLite engine actually being able to RUN the statement.
  // That gap shipped a bug: the emitted `SELECT * FROM $mutation_id` passed a
  // single RecordId where the engine's `selectByIds` lowering expects an array,
  // so it threw, the surrounding catch swallowed it at `error` level, and every
  // follower mutation was silently never pushed. Drive the real translator.
  it('discards a create with no payload instead of queueing a guaranteed failure', async () => {
    // `processUpEvent` does `Object.keys(event.data)`, so this row can never be
    // sent. Queued, it sat at the HEAD and blocked every later mutation for the
    // whole app: `next()` re-queues on failure, so the backlog never drained.
    const dropped: any[] = [];
    const rows: Record<string, unknown[]> = {
      [ROW.id]: [{ id: ROW.id, mutationType: 'create', recordId: 'comment:c1' }],
    };
    const query = vi.fn(async (sql: string, vars?: Record<string, unknown>) => {
      if (sql.startsWith('DELETE')) return [[]];
      const ids = (vars?.mutation_ids as unknown[]) ?? [];
      return [rows[String(ids[0] ?? '')] ?? []];
    });
    const queue = new UpQueue({ query } as any, makeLogger(), (d) => dropped.push(d));

    await queue.enqueueFromDatabase(ROW.id);

    expect(queue.size).toBe(0);
    expect(dropped).toHaveLength(1);
    // And the row is deleted, so it cannot re-poison the next boot.
    expect(query.mock.calls.some(([sql]) => String(sql).startsWith('DELETE'))).toBe(true);
  });

  it('loadFromDatabase drains the replayable rows even when one is unsendable', async () => {
    const dropped: any[] = [];
    const query = vi.fn(async (sql: string) => {
      if (sql.startsWith('DELETE')) return [[]];
      return [
        [
          { id: '_00_pending_mutations:a', mutationType: 'create', recordId: 'comment:c1' },
          {
            id: '_00_pending_mutations:b',
            mutationType: 'update',
            recordId: 'game:g1',
            data: { title: 'x' },
          },
        ],
      ];
    });
    const queue = new UpQueue({ query } as any, makeLogger(), (d) => dropped.push(d));

    await queue.loadFromDatabase();

    // The good row still loads; the poison one is dropped and reported.
    expect(queue.size).toBe(1);
    expect(dropped).toHaveLength(1);
    expect(dropped[0].mutationId).toBe('_00_pending_mutations:a');
  });

  it('persists the payload for a create, so it can be replayed at all', () => {
    // The create branch used to accept `dataVar` and ignore it, so the outbox
    // row was the ONLY copy of a pending create and it carried no data.
    const stmt = surql.createMutation('create', 'mid', 'id', 'data');
    expect(stmt).toContain('data = $data');
  });

  it('deletes the row using the STORED id, not its escaped display form', async () => {
    // Outbox ids contain `_` and `-`, so the store returns them escaped:
    // `_00_pending_mutations:⟨1785…_0005_c960…⟩`. Parsing that verbatim and
    // re-encoding escapes the brackets AGAIN, so the DELETE targeted
    // `⟨⟨…\⟩⟩` and matched nothing: a mutation replayed from the store could
    // never have its row removed, even after a SUCCESSFUL push, so it was
    // re-sent on every boot.
    const storedId = '_00_pending_mutations:⟨1785540216679_0005_c960958c-d6ed⟩';
    const deletes: any[] = [];
    const query = vi.fn(async (sql: string, vars?: Record<string, unknown>) => {
      if (sql.startsWith('DELETE')) {
        deletes.push(vars?.mutation_id);
        return [[]];
      }
      return [[{ id: storedId, mutationType: 'create', recordId: 'comment:c1' }]];
    });
    const queue = new UpQueue({ query } as any, makeLogger(), () => {});

    await queue.enqueueFromDatabase(storedId);

    expect(deletes).toHaveLength(1);
    const rid: any = deletes[0];
    expect(String(rid.table)).toBe('_00_pending_mutations');
    expect(rid.id).toBe('1785540216679_0005_c960958c-d6ed');
    expect(String(rid.id)).not.toContain('⟨');
  });

  it('a push timeout classifies as network, so it retries instead of rolling back', () => {
    const err = new Error('Mutation push timed out after 30000ms (create)');
    expect(classifySyncError(err)).toBe('network');
  });

  it('emits a statement the SQLite engine can actually translate', async () => {
    const { queue, query } = makeQueue({ [ROW.id]: [ROW] });
    await queue.enqueueFromDatabase(ROW.id);

    const [sql, vars] = query.mock.calls[0] as [string, Record<string, unknown>];
    const translated = translateSurql(sql, vars);
    const op: any = translated.ops[0];

    expect(op.kind).toBe('selectByIds');
    // The engine does `ids.length` then `ids.map(...)`: a non-array silently
    // skips the empty-guard and then throws.
    expect(Array.isArray(op.ids)).toBe(true);
    expect(op.ids).toHaveLength(1);
    expect(() => (op.ids as unknown[]).map((x) => x)).not.toThrow();
  });
});
