import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';

/**
 * `fn::query::heartbeat` is an `UPDATE $id SET ...`. Against a record that no
 * longer exists it matches nothing and returns an empty array — it does NOT
 * recreate the row. Verified against the deployed function:
 *
 *     RETURN fn::query::heartbeat(_00_query:definitely_not_a_real_row_xyz);
 *     -- (0 rows)
 *
 * So an unchecked heartbeat cannot tell "refreshed" from "the row I am
 * refreshing is gone", and a client whose row was reclaimed by the TTL sweep
 * beats against nothing forever: no membership, no edges, no re-registration.
 * The page then renders as though the data had been deleted — reported as
 * "Game not found" on a game that was open and working.
 *
 * This is reachable in ordinary use: the sweep expires on `lastActiveAt + ttl`
 * while the heartbeat runs on a timer browsers throttle hard in background
 * tabs, so a second window left idle past its TTL is the normal way in.
 */

function makeSync(heartbeatResult: unknown) {
  const logger: any = {
    child: () => logger,
    debug: () => {}, info: () => {}, warn: () => {}, error: () => {}, trace: () => {},
  };
  const remote: any = { query: vi.fn().mockResolvedValue(heartbeatResult) };
  const queryState: any = { config: { id: new RecordId('_00_query', 'h1') } };
  const dataModule: any = { getQueryByHash: vi.fn().mockReturnValue(queryState) };

  const sync = new Sp00kySync({} as any, remote, {} as any, dataModule, {} as any, logger);
  const enqueueDownEvent = vi.fn();
  (sync as any).enqueueDownEvent = enqueueDownEvent;

  return { sync, remote, dataModule, enqueueDownEvent };
}

describe('heartbeatQuery — noticing a reclaimed row', () => {
  beforeEach(() => vi.clearAllMocks());

  it('re-registers when the row it beat against is gone', async () => {
    // `UPDATE` on a deleted record: one statement, zero updated records.
    const { sync, enqueueDownEvent } = makeSync([[]]);

    await sync.heartbeatQuery('h1');

    expect(enqueueDownEvent).toHaveBeenCalledWith({
      type: 'register',
      payload: { hash: 'h1' },
    });
  });

  it('does nothing extra on a healthy heartbeat', async () => {
    const { sync, enqueueDownEvent } = makeSync([[{ id: 'x', lastActiveAt: 'now' }]]);

    await sync.heartbeatQuery('h1');

    expect(enqueueDownEvent).not.toHaveBeenCalled();
  });

  it('does not re-register on an unrecognised result shape', async () => {
    // Only an explicitly EMPTY update result means "the row is gone". Anything
    // else — a driver returning null, a shape change — must not be read as
    // deletion, or every heartbeat would re-register the whole working set.
    for (const shape of [null, undefined, [], [null], ['unexpected']]) {
      const { sync, enqueueDownEvent } = makeSync(shape);
      await sync.heartbeatQuery('h1');
      expect(enqueueDownEvent, `shape ${JSON.stringify(shape)}`).not.toHaveBeenCalled();
    }
  });

  it('still throws for a query that is no longer registered locally', async () => {
    const { sync, dataModule } = makeSync([[]]);
    dataModule.getQueryByHash.mockReturnValue(undefined);

    await expect(sync.heartbeatQuery('gone')).rejects.toThrow();
  });
});
