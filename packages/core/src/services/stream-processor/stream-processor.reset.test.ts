import { describe, it, expect, vi, beforeEach } from 'vitest';

// `StreamProcessorService.reset()` is the SSP half of a local-bucket switch:
// the old circuit holds the previous user's rows and views registered with the
// previous `$auth`, so reset must swap in a FRESH processor (no state load),
// and a checkpoint racing the reset must not persist the old circuit into the
// new bucket's store.

const processorInstances: any[] = [];

vi.mock('@spooky-sync/ssp-wasm', () => ({
  default: vi.fn(async () => {}),
  Sp00kyProcessor: vi.fn(() => {
    const instance = {
      ingest: vi.fn(() => []),
      register_view: vi.fn(() => ({ query_id: 'q1', result_data: [] })),
      unregister_view: vi.fn(),
      set_permissions: vi.fn(),
      set_projection: vi.fn(),
      save_store_state: vi.fn(() => new Uint8Array([1, 2, 3])),
      load_store_state: vi.fn(() => []),
      reconcile: vi.fn(() => ({ fetch: [], deleted: 0, updates: [] })),
      free: vi.fn(),
    };
    processorInstances.push(instance);
    return instance;
  }),
}));

import { StreamProcessorService } from './index';

const silentLogger = {
  child: () => silentLogger,
  trace: () => {},
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: () => {},
} as any;

function makeService() {
  const snapshots: Record<string, { bytes: Uint8Array; meta: any }> = {};
  const db = {
    epoch: 0,
    engineKind: 'sqlite' as const,
    scanVersions: vi.fn(async () => ({})),
    selectByIds: vi.fn(async () => []),
    getSnapshot: vi.fn(async (key: string) => snapshots[key] ?? null),
    putSnapshot: vi.fn(async (key: string, bytes: Uint8Array, meta: any) => {
      snapshots[key] = { bytes, meta };
    }),
  };
  const service = new StreamProcessorService({} as any, db as any, silentLogger);
  return { service, db, snapshots };
}

const primeCtx = { tables: [], schemaHash: 'schema-1', pendingIds: new Set<string>() };

beforeEach(() => {
  processorInstances.length = 0;
});

describe('StreamProcessorService.reset', () => {
  it('swaps in a fresh processor without loading persisted state', async () => {
    const { service } = makeService();
    await service.init();
    expect(processorInstances).toHaveLength(1);
    const first = processorInstances[0];

    await service.reset();
    expect(processorInstances).toHaveLength(2);
    const second = processorInstances[1];
    // Fresh circuit: nothing loaded into it (a persisted snapshot references
    // views under a dead sessionId salt and the previous user's data).
    expect(second.load_store_state).not.toHaveBeenCalled();

    // New registrations land on the fresh processor, not the old circuit.
    service.registerQueryPlan({
      queryHash: 'q1',
      surql: 'SELECT * FROM thing;',
      params: {},
      ttl: '10m',
      lastActiveAt: new Date(),
      localArray: [],
      remoteArray: [],
      meta: { tableName: 'thing' },
    } as any);
    expect(second.register_view).toHaveBeenCalled();
    expect(first.register_view).not.toHaveBeenCalled();
  });

  it('writes a store-only snapshot through the engine on checkpoint', async () => {
    const { service, db, snapshots } = makeService();
    service.configureCircuitPersistence(true);
    await service.init();
    await service.primeFromLocal(primeCtx);
    await service.checkpoint('test');
    expect(processorInstances[0].save_store_state).toHaveBeenCalledTimes(1);
    expect(db.putSnapshot).toHaveBeenCalledTimes(1);
    expect(snapshots.circuit.bytes).toEqual(new Uint8Array([1, 2, 3]));
    expect(snapshots.circuit.meta.schemaHash).toBe('schema-1');
    expect(snapshots.circuit.meta.formatVersion).toBe(1);
  });

  it('drops a checkpoint that raced a reset (old circuit never persists into the new bucket)', async () => {
    const { service, db } = makeService();
    service.configureCircuitPersistence(true);
    await service.init();
    await service.primeFromLocal(primeCtx);
    // The bytes are taken, then a reset lands before the write step,
    // simulated by resetting from inside save_store_state().
    processorInstances[0].save_store_state.mockImplementation(() => {
      void service.reset();
      return new Uint8Array([9]);
    });
    await service.checkpoint('test');
    expect(db.putSnapshot).not.toHaveBeenCalled();
  });

  it('does not write a snapshot from a follower tab', async () => {
    const { service, db } = makeService();
    service.configureCircuitPersistence(true);
    await service.init();
    await service.primeFromLocal(primeCtx);
    service.setPersistenceEnabled(false);
    await service.checkpoint('test');
    expect(db.putSnapshot).not.toHaveBeenCalled();
  });

  it('frees the replaced wasm circuit on reset', async () => {
    const { service } = makeService();
    await service.init();
    const first = processorInstances[0];
    await service.reset();
    // V8 cannot see wasm-internal bytes, so the FinalizationRegistry may never
    // run. The old circuit must be freed explicitly or its whole store stays
    // resident for the rest of the session.
    expect(first.free).toHaveBeenCalledTimes(1);
  });
});

// The renderer-OOM guardrail. A snapshot walks every row of the store, so it
// must never run inline with an ingest or a query register/unregister: on a
// large windowed list that meant hundreds of whole-store serializations per
// scroll. Snapshots are checkpointed on a timer, and only once enough rows
// changed to be worth the write.
describe('StreamProcessorService circuit snapshots', () => {
  const plan = {
    queryHash: 'q1',
    surql: 'SELECT * FROM thing;',
    params: {},
    ttl: '10m',
    lastActiveAt: new Date(),
    localArray: [],
    remoteArray: [],
    meta: { tableName: 'thing' },
  } as any;

  it('never snapshots on ingest or register/unregister when persistence is off', async () => {
    const { service, db } = makeService();
    service.configureCircuitPersistence(false);
    await service.init();
    const processor = processorInstances[0];

    service.registerQueryPlan(plan);
    service.ingest('thing', 'CREATE', 'thing:a', { id: 'thing:a' });
    service.ingestMany([
      { table: 'thing', op: 'CREATE', id: 'thing:b', record: { id: 'thing:b' } },
      { table: 'thing', op: 'CREATE', id: 'thing:c', record: { id: 'thing:c' } },
    ]);
    service.unregisterQueryPlan('q1');

    expect(processor.save_store_state).not.toHaveBeenCalled();
    expect(db.putSnapshot).not.toHaveBeenCalled();
  });

  it('still never snapshots inline when persistence is on', async () => {
    const { service, db } = makeService();
    service.configureCircuitPersistence(true, 30_000);
    await service.init();
    await service.primeFromLocal(primeCtx);
    const processor = processorInstances[0];

    service.registerQueryPlan(plan);
    service.ingest('thing', 'CREATE', 'thing:a', { id: 'thing:a' });
    service.unregisterQueryPlan('q1');

    // Marked dirty, but the write waits for the checkpoint interval.
    expect(processor.save_store_state).not.toHaveBeenCalled();
    expect(db.putSnapshot).not.toHaveBeenCalled();
    service.stopCheckpoints();
  });

  it('writes at most one snapshot per checkpoint tick, and none for a handful of rows', async () => {
    vi.useFakeTimers();
    try {
      const { service, db } = makeService();
      service.configureCircuitPersistence(true, 1000);
      await service.init();
      await service.primeFromLocal(primeCtx);
      const processor = processorInstances[0];

      // Below the row threshold: the tick stays quiet.
      for (let i = 0; i < 10; i++) {
        service.ingest('thing', 'CREATE', `thing:${i}`, { id: `thing:${i}` });
      }
      await vi.advanceTimersByTimeAsync(1000);
      expect(processor.save_store_state).not.toHaveBeenCalled();

      for (let i = 10; i < 60; i++) {
        service.ingest('thing', 'CREATE', `thing:${i}`, { id: `thing:${i}` });
      }
      await vi.advanceTimersByTimeAsync(1000);
      expect(processor.save_store_state).toHaveBeenCalledTimes(1);
      expect(db.putSnapshot).toHaveBeenCalledTimes(1);

      // Idle interval: nothing changed, so nothing is serialized.
      await vi.advanceTimersByTimeAsync(1000);
      expect(processor.save_store_state).toHaveBeenCalledTimes(1);

      service.stopCheckpoints();
    } finally {
      vi.useRealTimers();
    }
  });
});
