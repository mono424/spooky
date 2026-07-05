import { describe, it, expect, beforeEach, vi } from 'vitest';
import { DataModule } from './index';

/**
 * Tests for the recurring-outbox API added to DataModule: runRecurring builds a
 * single deterministic schedule row (idempotent, swallows a duplicate CREATE);
 * pokeRecurring bumps next_run_at only when the schedule exists; cancelRecurring
 * deletes it. The heavy create/update/delete pipeline is spied so these assert
 * the orchestration (deterministic id, field-building, existence gating) without
 * a real engine.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

const schema = {
  tables: [{ name: 'job', columns: {} }],
  backends: {
    gamesync: {
      outboxTable: 'job',
      routes: { '/syncGames': { args: { connection: { optional: false } } } },
    },
  },
};

const CONN = 'connection:CONN_abc';

// localQueryResult drives the existence check ([] = absent, [row] = present).
function makeDm(localQueryResult: unknown[]) {
  const local = { query: vi.fn().mockResolvedValue(localQueryResult) };
  const dm = new DataModule({} as any, local as any, schema as any, makeLogger(), 100);
  const create = vi.spyOn(dm, 'create').mockResolvedValue(undefined as any);
  const update = vi.spyOn(dm, 'update').mockResolvedValue(undefined as any);
  const del = vi.spyOn(dm, 'delete').mockResolvedValue(undefined as any);
  return { dm, local, create, update, del };
}

describe('DataModule.runRecurring', () => {
  it('creates a single deterministic schedule row with the recurring fields', async () => {
    const { dm, create } = makeDm([]); // no existing row
    await dm.runRecurring(
      'gamesync' as any,
      '/syncGames' as any,
      { connection: CONN } as any,
      { assignedTo: CONN, interval: 300000 }
    );

    expect(create).toHaveBeenCalledTimes(1);
    const [id, record] = create.mock.calls[0] as [string, any];
    expect(id.startsWith('job:')).toBe(true);
    expect(record.recurring).toBe(true);
    expect(record.interval).toBe(300000);
    expect(record.next_run_at).toBeInstanceOf(Date);
    expect(record.assigned_to).toBe(CONN);
    expect(record.path).toBe('/syncGames');
    expect(JSON.parse(record.payload)).toEqual({ connection: CONN });
  });

  it('is idempotent: does nothing when a schedule already exists', async () => {
    const { dm, create } = makeDm([{ id: 'job:x' }]); // existing row
    await dm.runRecurring(
      'gamesync' as any,
      '/syncGames' as any,
      { connection: CONN } as any,
      { assignedTo: CONN, interval: 300000 }
    );
    expect(create).not.toHaveBeenCalled();
  });

  it('swallows a duplicate CREATE (row exists on server but not yet synced locally)', async () => {
    const { dm, create } = makeDm([]); // local says absent
    create.mockRejectedValueOnce(new Error('record already exists'));
    await expect(
      dm.runRecurring(
        'gamesync' as any,
        '/syncGames' as any,
        { connection: CONN } as any,
        { assignedTo: CONN, interval: 300000 }
      )
    ).resolves.toBeUndefined();
  });

  it('uses a stable id per (assignedTo, path)', async () => {
    const a = makeDm([]);
    await a.dm.runRecurring('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, { assignedTo: CONN, interval: 300000 });
    const b = makeDm([]);
    await b.dm.runRecurring('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, { assignedTo: CONN, interval: 300000 });
    expect(a.create.mock.calls[0][0]).toBe(b.create.mock.calls[0][0]);
  });

  it('validates required options', async () => {
    const { dm } = makeDm([]);
    await expect(
      dm.runRecurring('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, { assignedTo: CONN } as any)
    ).rejects.toThrow(/interval/);
    await expect(
      dm.runRecurring('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, { interval: 300000 } as any)
    ).rejects.toThrow(/assignedTo/);
  });
});

describe('DataModule.pokeRecurring', () => {
  it('bumps next_run_at on the existing schedule row', async () => {
    const { dm, update } = makeDm([{ id: 'job:x' }]);
    await dm.pokeRecurring('gamesync' as any, '/syncGames' as any, { assignedTo: CONN });
    expect(update).toHaveBeenCalledTimes(1);
    const [table, id, data] = update.mock.calls[0] as [string, string, any];
    expect(table).toBe('job');
    expect(id.startsWith('job:')).toBe(true);
    expect(data.next_run_at).toBeInstanceOf(Date);
  });

  it('is a no-op when no schedule exists', async () => {
    const { dm, update } = makeDm([]);
    await dm.pokeRecurring('gamesync' as any, '/syncGames' as any, { assignedTo: CONN });
    expect(update).not.toHaveBeenCalled();
  });
});

describe('DataModule.cancelRecurring', () => {
  it('deletes the deterministic schedule row', async () => {
    const { dm, del, create } = makeDm([]);
    // Capture the id runRecurring would create, to prove cancel targets the same.
    await dm.runRecurring('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, { assignedTo: CONN, interval: 300000 });
    const createdId = create.mock.calls[0][0];

    await dm.cancelRecurring('gamesync' as any, '/syncGames' as any, { assignedTo: CONN });
    expect(del).toHaveBeenCalledTimes(1);
    const [table, id] = del.mock.calls[0] as [string, string];
    expect(table).toBe('job');
    expect(id).toBe(createdId);
  });
});
