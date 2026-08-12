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

  it('releases through fn::query::unsubscribe, never a bare DELETE', async () => {
    const { remote, run, queryId } = makeSync();

    await run('h1');

    expect(remote.query).toHaveBeenCalledWith('fn::query::unsubscribe($id)', {
      id: queryId,
    });
    // The regression that would blank other tabs' lists.
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

  it('re-registers if a subscriber reappeared during the release await', async () => {
    // Someone scrolled back / re-subscribed while the round trip was in flight.
    // Re-registering covers both outcomes: recreate the view if we were the
    // last subscriber, or re-add ourselves to `subscribers` if it survived.
    const { finalizeDeregister, enqueueDownEvent, run } = makeSync({
      hasSubscribers: [false, true],
    });

    await run('h1');

    expect(enqueueDownEvent).toHaveBeenCalledWith({
      type: 'register',
      payload: { hash: 'h1' },
    });
    expect(finalizeDeregister).not.toHaveBeenCalled();
  });

  it('is tolerant of an already torn-down query', async () => {
    const { remote, dataModule, run } = makeSync();
    dataModule.getQueryByHash.mockReturnValue(undefined);

    await expect(run('gone')).resolves.toBeUndefined();
    expect(remote.query).not.toHaveBeenCalled();
  });
});
