import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RecordId } from 'surrealdb';
import { Sp00kySync } from './sync';
import type { IngestTuple } from '../../services/tabs/protocol';

// Shared-tabs sync roles. A follower's optimistic write used to enter only
// its own circuit: the leader and every other follower learned of it after the
// server round-trip, through a LIVE event SurrealDB v3 sometimes drops. Now the
// follower posts the ingested tuples to the leader, which feeds its own circuit
// and fans them out (excluding the origin), and the leader tells every follower
// when a push settled so the row does not blink out of the render set.

function makeLogger(): any {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  return logger;
}

const tuple = (op: IngestTuple['op'], id = 'thread:c'): IngestTuple => ({
  table: 'thread',
  op,
  id,
  record: { id, _00_rv: 1 },
});

function makeSync() {
  const queryId = new RecordId('_00_query', 'h1');
  const queryState: any = {
    config: {
      id: queryId,
      localArray: [['thread:a', 1]],
      remoteArray: [['thread:a', 1]],
      membershipKnown: true,
      membershipKey: 'stable-key',
    },
  };
  const updateQueryRemoteArray = vi.fn(async (_h: string, next: any) => {
    queryState.config.remoteArray = next;
  });
  const dataModule: any = {
    getQueryById: vi.fn((id: RecordId) => (String(id.id) === 'h1' ? queryState : undefined)),
    getQueryByHash: vi.fn().mockReturnValue(queryState),
    updateQueryRemoteArray,
    notifyQuerySynced: vi.fn().mockResolvedValue(undefined),
    notifyTableQueries: vi.fn().mockResolvedValue(undefined),
    noteWriteSettled: vi.fn(),
    getActiveQueryHashes: () => ['h1'],
    getPendingRecordIds: async () => ({ writes: new Set(), deletes: new Set() }),
  };
  const cache: any = { applyRelayedIngest: vi.fn() };
  const sync = new Sp00kySync(
    {} as any,
    { query: vi.fn() } as any,
    cache,
    dataModule,
    {} as any,
    makeLogger()
  );
  (sync as any).runSyncForQuery = vi.fn().mockResolvedValue(undefined);
  (sync as any).upQueue.enqueueFromDatabase = vi.fn().mockResolvedValue(undefined);
  (sync as any).scheduler.enqueueMutation = vi.fn();

  const hub: any = {
    onFollowerMessage: null,
    relayIngest: vi.fn(),
    broadcast: vi.fn(),
    sendTo: vi.fn(),
  };
  const forwarder: any = {
    onLeaderMessage: null,
    mutationEnqueued: vi.fn(),
    ingest: vi.fn(),
  };
  return { sync, cache, dataModule, hub, forwarder, queryState, updateQueryRemoteArray };
}

describe('shared-tabs leader', () => {
  beforeEach(() => vi.clearAllMocks());

  it('applies a follower ingest to its own circuit and relays it to the other followers', () => {
    const { sync, cache, hub } = makeSync();
    sync.promoteToLeader(hub);
    const tuples = [tuple('CREATE')];

    hub.onFollowerMessage('tab-2', { type: 'ingest', tuples });

    expect(cache.applyRelayedIngest).toHaveBeenCalledWith(tuples);
    expect(hub.relayIngest).toHaveBeenCalledWith(tuples, 'tab-2');
  });

  it('re-materializes the table queries for a relayed DELETE', async () => {
    const { sync, dataModule, hub } = makeSync();
    sync.promoteToLeader(hub);

    hub.onFollowerMessage('tab-2', { type: 'ingest', tuples: [tuple('DELETE')] });
    await Promise.resolve();

    expect(dataModule.notifyTableQueries).toHaveBeenCalledWith('thread');
  });

  it('does not re-materialize for a relayed CREATE/UPDATE (the stream update covers it)', async () => {
    const { sync, dataModule, hub } = makeSync();
    sync.promoteToLeader(hub);

    hub.onFollowerMessage('tab-2', { type: 'ingest', tuples: [tuple('UPDATE')] });
    await Promise.resolve();

    expect(dataModule.notifyTableQueries).not.toHaveBeenCalled();
  });

  it('treats a follower write as activity for the poll backoff', () => {
    const { sync, hub } = makeSync();
    sync.promoteToLeader(hub);

    (sync as any).listRefIdleStreak = 7;
    hub.onFollowerMessage('tab-2', { type: 'ingest', tuples: [tuple('CREATE')] });
    expect((sync as any).listRefIdleStreak).toBe(0);

    (sync as any).listRefIdleStreak = 7;
    hub.onFollowerMessage('tab-2', {
      type: 'mutation-enqueued',
      mutationId: '_00_pending_mutations:1_0001_tab-2',
    });
    expect((sync as any).listRefIdleStreak).toBe(0);
    expect((sync as any).upQueue.enqueueFromDatabase).toHaveBeenCalledWith(
      '_00_pending_mutations:1_0001_tab-2'
    );
  });

  it('notes a settled mutation locally and broadcasts it to every follower', () => {
    const { sync, dataModule, hub } = makeSync();
    sync.promoteToLeader(hub);

    (sync as any).handleMutationSettled({
      type: 'create',
      mutation_id: new RecordId('_00_pending_mutations', '1_0001_tab-2'),
      record_id: new RecordId('thread', 'c'),
    });

    expect(dataModule.noteWriteSettled).toHaveBeenCalledWith('thread:c', 'create');
    expect(hub.broadcast).toHaveBeenCalledWith({
      type: 'mutation-settled',
      mutationId: '_00_pending_mutations:1_0001_tab-2',
      recordId: 'thread:c',
      eventType: 'create',
    });
  });

  it('solo: a settled mutation is noted locally with no hub to broadcast on', () => {
    const { sync, dataModule } = makeSync();
    (sync as any).handleMutationSettled({
      type: 'update',
      mutation_id: new RecordId('_00_pending_mutations', '1_0001_solo'),
      record_id: new RecordId('thread', 'c'),
    });
    expect(dataModule.noteWriteSettled).toHaveBeenCalledWith('thread:c', 'update');
  });
});

describe('shared-tabs follower', () => {
  beforeEach(() => vi.clearAllMocks());

  it('starts the local settled-write grace on mutation-settled', () => {
    const { sync, dataModule, forwarder } = makeSync();
    sync.demoteToFollower(forwarder);

    forwarder.onLeaderMessage({
      type: 'mutation-settled',
      mutationId: '_00_pending_mutations:1_0001_tab-1',
      recordId: 'thread:c',
      eventType: 'delete',
    });

    expect(dataModule.noteWriteSettled).toHaveBeenCalledWith('thread:c', 'delete');
  });

  it('feeds a relayed ingest to the local circuit', async () => {
    const { sync, cache, dataModule, forwarder } = makeSync();
    sync.demoteToFollower(forwarder);
    const tuples = [tuple('DELETE')];

    forwarder.onLeaderMessage({ type: 'ingest-relay', tuples, leadershipId: 1, seq: 1 });
    await Promise.resolve();

    expect(cache.applyRelayedIngest).toHaveBeenCalledWith(tuples);
    expect(dataModule.notifyTableQueries).toHaveBeenCalledWith('thread');
  });

  it('applies a relayed list_ref change for one of its own queries', async () => {
    const { sync, forwarder, updateQueryRemoteArray } = makeSync();
    sync.demoteToFollower(forwarder);

    forwarder.onLeaderMessage({
      type: 'list-ref-change',
      action: 'CREATE',
      queryId: '_00_query:h1',
      recordId: 'thread:c',
      version: 1,
      parent: false,
    });
    await new Promise((r) => setTimeout(r, 0));

    expect(updateQueryRemoteArray).toHaveBeenCalledWith('h1', [
      ['thread:a', 1],
      ['thread:c', 1],
    ]);
    expect((sync as any).runSyncForQuery).toHaveBeenCalled();
  });

  it('ignores a relayed list_ref change for another tab query', async () => {
    const { sync, forwarder, updateQueryRemoteArray } = makeSync();
    sync.demoteToFollower(forwarder);

    forwarder.onLeaderMessage({
      type: 'list-ref-change',
      action: 'CREATE',
      queryId: '_00_query:foreign',
      recordId: 'thread:c',
      version: 1,
      parent: false,
    });
    await new Promise((r) => setTimeout(r, 0));

    expect(updateQueryRemoteArray).not.toHaveBeenCalled();
    expect((sync as any).runSyncForQuery).not.toHaveBeenCalled();
  });

  it('forwards mutation ids to the leader instead of queueing locally', async () => {
    const { sync, forwarder } = makeSync();
    sync.demoteToFollower(forwarder);

    await sync.enqueueMutation([
      {
        type: 'create',
        mutation_id: new RecordId('_00_pending_mutations', '1_0001_tab-2'),
        record_id: new RecordId('thread', 'c'),
      } as any,
    ]);

    expect(forwarder.mutationEnqueued).toHaveBeenCalledWith('_00_pending_mutations:1_0001_tab-2');
    expect((sync as any).scheduler.enqueueMutation).not.toHaveBeenCalled();
  });
});
