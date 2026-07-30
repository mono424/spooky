import { describe, it, expect, vi } from 'vitest';
import { UpQueue } from './queue-up';

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
      const id = String(vars?.mutation_id ?? '');
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
});
