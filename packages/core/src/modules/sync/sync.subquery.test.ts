import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';
import { buildSubqueryListRefSelect } from './utils';
import { encodeRecordId } from '../../utils/index';

// Guards the CLIENT half of `.related()` (author/comments) delivery: given
// subquery-child edges on the server (`_00_list_ref…` rows with
// `parent IS NOT NONE`), `syncSubqueryChildren` MUST pull the child bodies
// through the sync engine so a later re-materialization attaches them to the
// parent. If this stops forwarding the child ids, related fields come back
// empty (threads render with an "Anonymous" author, comments vanish) even
// though the server wrote the edges correctly.

function makeSync() {
  const logger: any = {
    child: () => logger,
    debug: () => {}, info: () => {}, warn: () => {}, error: () => {}, trace: () => {},
  };
  const remote: any = { query: vi.fn() };
  const queryId = new RecordId('_00_query', 'h1');
  const queryState: any = { config: { id: queryId, subqueryRemoteArray: [] } };
  const dataModule: any = { getQueryByHash: vi.fn().mockReturnValue(queryState) };

  const sync = new Sp00kySync(
    {} as any, remote, {} as any, dataModule, {} as any, logger,
  );

  // syncEngine is constructed internally; replace it with a spy.
  const syncRecords = vi.fn().mockResolvedValue(undefined);
  (sync as any).syncEngine = { syncRecords };
  // Pin the resolved per-user list_ref table (auth routing is out of scope here).
  (sync as any).listRefTable = () => '_00_list_ref_user_x';

  const run = (hash: string) => (sync as any).syncSubqueryChildren(hash) as Promise<void>;
  return { remote, syncRecords, queryState, queryId, run };
}

describe('syncSubqueryChildren — related-child delivery', () => {
  beforeEach(() => vi.clearAllMocks());

  it('reads the subquery edges (parent IS NOT NONE) scoped to the query id', async () => {
    const { remote, run, queryId } = makeSync();
    remote.query.mockResolvedValue([[]]);

    await run('h1');

    expect(remote.query).toHaveBeenCalledWith(
      buildSubqueryListRefSelect('_00_list_ref_user_x'),
      { in: queryId }
    );
  });

  it('forwards each subquery child id to the sync engine so its body is cached', async () => {
    const { remote, syncRecords, run } = makeSync();
    // Server has one author child edge for this query.
    remote.query.mockResolvedValue([[
      { out: new RecordId('user', 'u'), version: 1 },
    ]]);

    await run('h1');

    expect(syncRecords).toHaveBeenCalledTimes(1);
    const arg = syncRecords.mock.calls[0][0];
    expect(arg.added).toHaveLength(1);
    expect(encodeRecordId(arg.added[0].id)).toBe('user:u');
    expect(arg.added[0].version).toBe(1);
    expect(arg.removed).toEqual([]); // shared child bodies are never deleted here
  });

  it('is idempotent: unchanged edges do not refetch bodies', async () => {
    const { remote, syncRecords, run, queryState } = makeSync();
    queryState.config.subqueryRemoteArray = [['user:u', 1]];
    remote.query.mockResolvedValue([[
      { out: new RecordId('user', 'u'), version: 1 },
    ]]);

    await run('h1');

    expect(syncRecords).not.toHaveBeenCalled();
  });
});
