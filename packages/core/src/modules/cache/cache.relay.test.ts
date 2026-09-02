import { describe, it, expect, vi } from 'vitest';
import { RecordId } from 'surrealdb';
import { CacheModule } from './index';

// The ingest relay is how one tab's circuit learns what another tab wrote.
// The leader relays every ingest (its sync fetches are the only copy the
// followers get); a follower relays with `localWritesOnly`, just its mutation
// path, because its sync-fetched batches are the leader's data coming back.

function makeLogger(): any {
  const logger: any = { debug: () => {}, info: () => {}, warn: () => {}, error: () => {}, trace: () => {} };
  logger.child = () => logger;
  return logger;
}

function setup() {
  const local: any = { epoch: 1, execute: vi.fn(async () => {}), query: vi.fn(async () => []) };
  const ssp: any = {
    addReceiver: vi.fn(),
    ingestMany: vi.fn((records: unknown[]) => records),
  };
  const cache = new CacheModule(local, ssp, () => {}, makeLogger());
  const relay = vi.fn();
  return { cache, local, ssp, relay };
}

const rec = (id: string, version: number) => ({
  table: 'thread',
  op: 'CREATE' as const,
  record: { id: new RecordId('thread', id), title: 't' },
  version,
});

describe('CacheModule ingest relay', () => {
  it('applyRelayedIngest updates the version memo and never re-relays', () => {
    const { cache, ssp, relay } = setup();
    cache.setIngestRelay(relay);

    cache.applyRelayedIngest([
      { table: 'thread', op: 'CREATE', id: 'thread:a', record: { id: 'thread:a', _00_rv: 4 } },
    ]);

    expect(ssp.ingestMany).toHaveBeenCalledTimes(1);
    expect(cache.lookup('thread:a')).toBe(4);
    expect(relay).not.toHaveBeenCalled();
  });

  it('localWritesOnly relays the mutation path (save + delete)', async () => {
    const { cache, local, relay } = setup();
    cache.setIngestRelay(relay, { localWritesOnly: true });

    await cache.save(rec('a', 1), true);
    expect(local.execute).not.toHaveBeenCalled();
    expect(relay).toHaveBeenCalledTimes(1);
    expect(relay.mock.calls[0][0][0]).toMatchObject({ table: 'thread', op: 'CREATE', id: 'thread:a' });

    const before = { id: 'thread:a', title: 't' };
    await cache.delete('thread', 'thread:a', true, before);
    expect(relay).toHaveBeenCalledTimes(2);
    expect(relay.mock.calls[1][0]).toEqual([
      { table: 'thread', op: 'DELETE', id: 'thread:a', record: before },
    ]);
  });

  it('localWritesOnly stays silent for sync-fetched batches', async () => {
    const { cache, local, relay } = setup();
    cache.setIngestRelay(relay, { localWritesOnly: true });

    await cache.saveBatch([rec('a', 1), rec('b', 1)], false);
    expect(local.execute).toHaveBeenCalledTimes(1);
    expect(relay).not.toHaveBeenCalled();

    await cache.delete('thread', 'thread:a', false, {});
    expect(relay).not.toHaveBeenCalled();
  });

  it('an unscoped relay fires for every ingest', async () => {
    const { cache, relay } = setup();
    cache.setIngestRelay(relay);

    await cache.saveBatch([rec('a', 1)], false);
    await cache.save(rec('b', 1), true);
    await cache.delete('thread', 'thread:a', false, {});

    expect(relay).toHaveBeenCalledTimes(3);
  });

  it('setIngestRelay(null) stops relaying', async () => {
    const { cache, relay } = setup();
    cache.setIngestRelay(relay, { localWritesOnly: true });
    cache.setIngestRelay(null);
    await cache.save(rec('a', 1), true);
    expect(relay).not.toHaveBeenCalled();
  });
});
