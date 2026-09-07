import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Sp00kySync } from './sync';

// The idle list-ref poll is the only health signal that runs on a quiet page.
// These tests exercise `pollListRefForActiveQueries` -> `recordSyncOutcome` in
// isolation: a run of network-failed cycles degrades, and a single clean cycle
// recovers WITHOUT any mutation (the reported bug). Construction is cheap —
// Sp00kySync's constructor only stores refs and calls logger.child; no timers or
// I/O start until init(), which we never call.

const CONNECTION_UNAVAILABLE =
  'You must be connected to a SurrealDB instance before performing this operation';

function makeSync(hashes: string[]) {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const remote: any = { query: vi.fn().mockResolvedValue([true]) };
  const dataModule: any = { getActiveQueryHashes: () => hashes, getQueryByHash: () => undefined };

  const sync = new Sp00kySync(
    {} as any,
    remote,
    {} as any,
    dataModule,
    {} as any,
    logger,
    { degradeAfterConsecutiveFailures: 3 }
  );

  // The poll reads every chunk of active hashes in one round trip
  // (`fetchListRefSnapshots`) and lands each query's snapshot separately
  // (`applyListRefSnapshot`). Stub both so the test controls per-cycle
  // reachability. `poll` invokes the private method.
  const fetch = vi.fn();
  const apply = vi.fn().mockResolvedValue(false);
  (sync as any).fetchListRefSnapshots = fetch;
  (sync as any).applyListRefSnapshot = apply;
  const snapshotsFor = (hs: string[]) =>
    new Map(hs.map((h) => [h, { primary: [], subquery: [], rowCount: null }]));
  fetch.mockImplementation(async (hs: string[]) => snapshotsFor(hs));
  const poll = () => (sync as any).pollListRefForActiveQueries() as Promise<boolean>;

  return { sync, remote, fetch, apply, snapshotsFor, poll };
}

describe('sync health via idle poll', () => {
  beforeEach(() => vi.clearAllMocks());

  it('degrades after N consecutive network-failed poll cycles', async () => {
    const { sync, fetch, poll } = makeSync(['h1']);
    fetch.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));

    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
    await poll();
    expect(sync.syncHealth.status).toBe('degraded');
    expect(sync.syncHealth.kind).toBe('network');
  });

  it('recovers on the next clean poll cycle — no mutation needed', async () => {
    const { sync, fetch, snapshotsFor, poll } = makeSync(['h1']);
    fetch.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));
    await poll();
    await poll();
    await poll();
    expect(sync.syncHealth.status).toBe('degraded');

    // Connectivity returns; a plain idle poll (no user action) clears it.
    fetch.mockImplementation(async (hs: string[]) => snapshotsFor(hs));
    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
  });

  it('does not degrade on application errors (server was reached)', async () => {
    const { sync, fetch, poll } = makeSync(['h1']);
    fetch.mockRejectedValue(new Error('Permission denied'));
    await poll();
    await poll();
    await poll();
    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
  });

  it('counts a cycle whose round trip answered as reached even when a per-query apply fails', async () => {
    const { sync, fetch, apply, snapshotsFor, poll } = makeSync(['h1', 'h2']);
    // First degrade via all-network cycles.
    fetch.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));
    await poll();
    await poll();
    await poll();
    expect(sync.syncHealth.status).toBe('degraded');

    // The batched read answers (server reached); landing one of the two
    // snapshots then fails on its own network error → still a reached cycle.
    fetch.mockImplementation(async (hs: string[]) => snapshotsFor(hs));
    apply
      .mockResolvedValueOnce(true)
      .mockRejectedValueOnce(new Error(CONNECTION_UNAVAILABLE));
    await expect(poll()).resolves.toBe(true);
    expect(sync.syncHealth.status).toBe('healthy');
    expect(fetch).toHaveBeenLastCalledWith(['h1', 'h2']);
  });

  it('probes RETURN true when there are no active queries', async () => {
    const { sync, remote, poll } = makeSync([]);
    remote.query.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));
    await poll();
    await poll();
    await poll();
    expect(remote.query).toHaveBeenCalledWith('RETURN true');
    expect(sync.syncHealth.status).toBe('degraded');

    remote.query.mockResolvedValue([true]);
    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
  });

  it('leaves everConnected false through a cold-start failure run', async () => {
    const { sync, fetch, poll } = makeSync(['h1']);
    expect(sync.syncHealth.everConnected).toBe(false);

    // Server never reached: 3 failed cycles degrade, but this is the initial
    // "connecting" phase, not a lost connection — everConnected stays false.
    fetch.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));
    await poll();
    await poll();
    await poll();
    expect(sync.syncHealth.status).toBe('degraded');
    expect(sync.syncHealth.everConnected).toBe(false);
  });

  it('latches everConnected on the first success and keeps it through a later degrade', async () => {
    const { sync, fetch, poll } = makeSync(['h1']);

    // First successful round reaches the server: connecting phase is over.
    await poll();
    expect(sync.syncHealth.status).toBe('healthy');
    expect(sync.syncHealth.everConnected).toBe(true);

    // Connection later drops: degraded now reflects a REAL lost connection.
    fetch.mockReset();
    fetch.mockRejectedValue(new Error(CONNECTION_UNAVAILABLE));
    await poll();
    await poll();
    await poll();
    expect(sync.syncHealth.status).toBe('degraded');
    expect(sync.syncHealth.everConnected).toBe(true);
  });
});
