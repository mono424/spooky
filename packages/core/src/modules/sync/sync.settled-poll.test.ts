import { describe, it, expect, vi, afterEach } from 'vitest';
import { Sp00kySync } from './sync';

// The list_ref poll backs off toward a 5s cap on a quiet page. A settled write
// that is still waiting for its membership edge must hold the poll at its base
// cadence instead: the row is rendered on the settled-write grace alone, so
// the edge has to be found on the first poll after it lands, not after the
// backoff has grown. Otherwise a missed LIVE notification during a server
// stall turns into "my message vanished" even though the edge arrived.

function makeSync(pending: () => boolean) {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const remote: any = { query: vi.fn().mockResolvedValue([true]) };
  const dataModule: any = {
    getActiveQueryHashes: () => ['h1'],
    hasSettledWritesPending: pending,
  };
  const sync = new Sp00kySync({} as any, remote, {} as any, dataModule, {} as any, logger);
  // A poll that never observes a change: the only thing driving the streak is
  // the settled-write signal.
  (sync as any).pollListRefForActiveQueries = vi.fn().mockResolvedValue(false);
  return sync;
}

describe('list_ref poll cadence while a settled write awaits membership', () => {
  afterEach(() => vi.useRealTimers());

  it('stays at the base cadence while a write is pending, then backs off', async () => {
    vi.useFakeTimers();
    let pending = true;
    const sync = makeSync(() => pending);
    const base = sync.refSyncIntervalMs;

    (sync as any).startListRefPoll();

    // Three quiet ticks with a write outstanding: no backoff at all.
    for (let i = 0; i < 3; i++) {
      await vi.advanceTimersByTimeAsync(base);
      expect((sync as any).listRefIdleStreak).toBe(0);
    }

    // Membership caught up: the ordinary idle backoff resumes.
    pending = false;
    await vi.advanceTimersByTimeAsync(base);
    expect((sync as any).listRefIdleStreak).toBe(1);
    await vi.advanceTimersByTimeAsync(base * 2);
    expect((sync as any).listRefIdleStreak).toBe(2);

    (sync as any).stopListRefPoll();
  });
});
