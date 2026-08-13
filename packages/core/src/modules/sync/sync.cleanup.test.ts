import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

// Guards the teardown half of shared views. A `_00_query` row can be shared by
// several sessions of the same user, so releasing it MUST go through
// `fn::query::unsubscribe` — which drops only this session from `subscribers`
// and deletes the row when it was the last one. A bare `DELETE $id` here would
// destroy the view and every `_00_list_ref` edge hanging off it for every other
// live tab, which reads to the user as a list that silently goes blank.
//
// This was previously unenforceable: `_00_query` granted no delete permission,
// so the old `DELETE $id` affected zero rows and did nothing at all. Now that
// the table grants delete, a regression here is destructive rather than inert.

function makeSync(opts: { hasSubscribers?: boolean[] } = {}) {
  const logger: any = {
    child: () => logger,
    debug: () => {}, info: () => {}, warn: () => {}, error: () => {}, trace: () => {},
  };
  const remote: any = { query: vi.fn().mockResolvedValue([{ released: true, remaining: 0 }]) };
  const queryId = new RecordId('_00_query', 'h1');
  const queryState: any = { config: { id: queryId } };

  // Successive hasSubscribers() answers: [before the release, after it].
  const answers = opts.hasSubscribers ?? [false, false];
  let call = 0;
  const hasSubscribers = vi.fn(() => answers[Math.min(call++, answers.length - 1)]);

  const finalizeDeregister = vi.fn();
  const dataModule: any = {
    getQueryByHash: vi.fn().mockReturnValue(queryState),
    hasSubscribers,
    finalizeDeregister,
  };

  const sync = new Sp00kySync(
    {} as any, remote, {} as any, dataModule, {} as any, logger,
  );
  const enqueueDownEvent = vi.fn();
  (sync as any).enqueueDownEvent = enqueueDownEvent;

  const run = (hash: string) => (sync as any).cleanupQuery(hash) as Promise<void>;
  return { remote, dataModule, queryId, finalizeDeregister, enqueueDownEvent, run };
}

describe('cleanupQuery — releasing a possibly-shared view', () => {
  beforeEach(() => vi.clearAllMocks());

  it('does not touch the remote view at all; the TTL sweep reclaims it', async () => {
    // Eager release is deliberately disabled. It was inert for months (the
    // table granted no delete permission), and making it real turned every
    // best-effort guard misfire into a live delete of the row and all its
    // edges. TTL remains the only reclamation that has actually run.
    const { remote, run } = makeSync();

    await run('h1');

    expect(remote.query).not.toHaveBeenCalled();
  });

  it('never issues a bare DELETE', async () => {
    // Belt and braces: if the eager path is ever re-enabled, it must go through
    // the refcounted `fn::query::unsubscribe`, never a raw DELETE, which would
    // tear the view out from under other sessions sharing the row.
    const { remote, run } = makeSync();

    await run('h1');

    const sql = remote.query.mock.calls.map((c: any[]) => c[0]).join('\n');
    expect(sql).not.toMatch(/\bDELETE\b/i);
  });

  it('frees local state once released', async () => {
    const { finalizeDeregister, enqueueDownEvent, run } = makeSync();

    await run('h1');

    expect(finalizeDeregister).toHaveBeenCalledWith('h1');
    expect(enqueueDownEvent).not.toHaveBeenCalled();
  });

  it('does not touch the remote row if a subscriber reappeared before the release', async () => {
    const { remote, finalizeDeregister, run } = makeSync({
      hasSubscribers: [true],
    });

    await run('h1');

    expect(remote.query).not.toHaveBeenCalled();
    expect(finalizeDeregister).not.toHaveBeenCalled();
  });

  it('still frees local state when a subscriber reappears mid-cleanup', async () => {
    // With no remote round trip there is no window to lose a re-subscribe in,
    // so this collapses to the ordinary local free. The remote view survives
    // regardless (TTL owns it), which is precisely why the reappearing
    // subscriber is safe: a re-register finds the row still there.
    const { remote, finalizeDeregister, run } = makeSync({
      hasSubscribers: [false, true],
    });

    await run('h1');

    expect(remote.query).not.toHaveBeenCalled();
    expect(finalizeDeregister).toHaveBeenCalledWith('h1');
  });

  it('is tolerant of an already torn-down query', async () => {
    const { remote, dataModule, run } = makeSync();
    dataModule.getQueryByHash.mockReturnValue(undefined);

    await expect(run('gone')).resolves.toBeUndefined();
    expect(remote.query).not.toHaveBeenCalled();
  });
});
