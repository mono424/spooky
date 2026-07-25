import { describe, it, expect, vi } from 'vitest';
import { DataModule } from './index';

/**
 * Tests for `DataModule.run`, the one-shot outbox API.
 *
 * Recurring jobs used to live here too (`runRecurring` / `pokeRecurring` /
 * `cancelRecurring`, which wrote a single durable row the runner re-armed
 * forever). They are gone: schedules are now declared server-side under
 * `schedules:` in sp00ky.yml, and the scheduler creates a fresh job row per
 * cycle — so every row this API writes is exactly one execution.
 *
 * The create/update pipeline is spied, so these assert the orchestration
 * (table resolution, argument validation, field building) without a real engine.
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
      routes: {
        '/syncGames': {
          args: { connection: { optional: false }, since: { optional: true } },
        },
      },
    },
    noOutbox: {
      routes: { '/whatever': { args: {} } },
    },
  },
};

const CONN = 'connection:CONN_abc';

function makeDm() {
  const local = { query: vi.fn().mockResolvedValue([]) };
  const dm = new DataModule({} as any, local as any, schema as any, makeLogger(), 100);
  const create = vi.spyOn(dm, 'create').mockResolvedValue(undefined as any);
  return { dm, local, create };
}

describe('DataModule.run', () => {
  it('creates the one-shot job with status pending so the optimistic row reads in-flight', async () => {
    const { dm, create } = makeDm();
    await dm.run('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, {
      assignedTo: CONN,
    });

    expect(create).toHaveBeenCalledTimes(1);
    const [id, record] = create.mock.calls[0] as [string, any];
    expect(id.startsWith('job:')).toBe(true);
    // The schema's DEFAULT ALWAYS "pending" only runs server-side; without the
    // explicit field the local optimistic row has status undefined and
    // in-flight indicators miss it until the first server echo.
    expect(record.status).toBe('pending');
  });

  it('gives each call its own row', async () => {
    const { dm, create } = makeDm();
    await dm.run('gamesync' as any, '/syncGames' as any, { connection: CONN } as any);
    await dm.run('gamesync' as any, '/syncGames' as any, { connection: CONN } as any);

    const [firstId] = create.mock.calls[0] as [string, any];
    const [secondId] = create.mock.calls[1] as [string, any];
    expect(firstId).not.toBe(secondId);
  });

  it('carries the retry, timeout and delay options onto the row', async () => {
    const { dm, create } = makeDm();
    await dm.run('gamesync' as any, '/syncGames' as any, { connection: CONN } as any, {
      assignedTo: CONN,
      max_retries: 5,
      retry_strategy: 'exponential',
      timeout: 30,
      delay: 60_000,
    });

    const [, record] = create.mock.calls[0] as [string, any];
    expect(record.max_retries).toBe(5);
    expect(record.retry_strategy).toBe('exponential');
    expect(record.timeout).toBe(30);
    expect(record.delay).toBe(60_000);
  });

  it('serializes the payload and rejects a missing required argument', async () => {
    const { dm, create } = makeDm();
    await dm.run('gamesync' as any, '/syncGames' as any, { connection: CONN } as any);
    const [, record] = create.mock.calls[0] as [string, any];
    expect(JSON.parse(record.payload)).toEqual({ connection: CONN });

    await expect(
      dm.run('gamesync' as any, '/syncGames' as any, {} as any)
    ).rejects.toThrow(/connection/);
  });

  it('rejects an unknown route and a backend with no outbox table', async () => {
    const { dm } = makeDm();
    await expect(
      dm.run('gamesync' as any, '/nope' as any, {} as any)
    ).rejects.toThrow(/not found/);
    await expect(
      dm.run('noOutbox' as any, '/whatever' as any, {} as any)
    ).rejects.toThrow(/Outbox table/);
  });
});
