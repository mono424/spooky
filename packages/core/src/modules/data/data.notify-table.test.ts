import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { DataModule } from './index';

// A DELETE that landed in the local store (this tab's own, or one relayed
// from another tab) needs a forced re-materialize of the table's queries: the
// SSP may not emit a view update for it.

function makeLogger(): any {
  const logger: any = { debug: () => {}, info: () => {}, warn: () => {}, error: () => {}, trace: () => {} };
  logger.child = () => logger;
  return logger;
}

const schema = { tables: [{ name: 'comment', columns: {} }, { name: 'thread', columns: {} }] } as any;

function state(hash: string, tableName: string): any {
  return {
    config: { id: new RecordId('_00_query', hash), tableName, localArray: [], remoteArray: [] },
    records: [],
  };
}

describe('DataModule.notifyTableQueries', () => {
  it('re-materializes only the queries on that table, isolating failures', async () => {
    const local: any = { epoch: 1, query: vi.fn(async () => [[]]) };
    const dm = new DataModule({ saveBatch: async () => {} } as any, local, schema, makeLogger(), 100);
    (dm as any).activeQueries.set('c1', state('c1', 'comment'));
    (dm as any).activeQueries.set('c2', state('c2', 'comment'));
    (dm as any).activeQueries.set('t1', state('t1', 'thread'));
    const notified: string[] = [];
    dm.notifyQuerySynced = vi.fn(async (hash: string) => {
      notified.push(hash);
      if (hash === 'c1') throw new Error('boom');
    }) as any;

    await dm.notifyTableQueries('comment');

    expect(notified.sort()).toEqual(['c1', 'c2']);
  });
});
