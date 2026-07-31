import { describe, it, expect, vi, beforeEach } from 'vitest';

// `StreamProcessorService.reset()` is the SSP half of a local-bucket switch:
// the old circuit holds the previous user's rows and views registered with the
// previous `$auth`, so reset must swap in a FRESH processor (no state load),
// and a `saveState` racing the reset must not persist the old circuit into the
// new bucket's key.

const processorInstances: any[] = [];

vi.mock('@spooky-sync/ssp-wasm', () => ({
  default: vi.fn(async () => {}),
  Sp00kyProcessor: vi.fn(() => {
    const instance = {
      ingest: vi.fn(() => []),
      register_view: vi.fn(() => ({ query_id: 'q1', result_data: [] })),
      unregister_view: vi.fn(),
      set_permissions: vi.fn(),
      save_state: vi.fn(() => 'state-bytes'),
      load_state: vi.fn(),
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
  const persisted: Record<string, unknown> = {};
  const persistence = {
    set: vi.fn(async (key: string, value: unknown) => {
      persisted[key] = value;
    }),
    get: vi.fn(async () => null),
    remove: vi.fn(async () => {}),
  };
  const service = new StreamProcessorService(
    {} as any,
    {} as any,
    persistence as any,
    silentLogger
  );
  return { service, persistence, persisted };
}

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
    expect(second.load_state).not.toHaveBeenCalled();

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

  it('routes persisted state to the per-bucket key', async () => {
    const { service, persistence } = makeService();
    service.configureCircuitPersistence(true);
    await service.init();
    service.setStateKeySuffix('u1');
    await service.saveState();
    expect(persistence.set).toHaveBeenCalledWith('_00_stream_processor_state:u1', 'state-bytes');
  });

  it('drops a saveState that raced a reset (old circuit never persists into the new key)', async () => {
    const { service, persistence } = makeService();
    service.configureCircuitPersistence(true);
    await service.init();
    // The snapshot is taken, then a reset lands before the persist step,
    // simulated by bumping the generation from inside save_state().
    processorInstances[0].save_state.mockImplementation(() => {
      void service.reset();
      return 'stale-circuit';
    });
    await service.saveState();
    expect(persistence.set).not.toHaveBeenCalled();
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

// The renderer-OOM guardrail. `Circuit::save` deep-clones the entire store (every
// row of every ingested table) and JSON-encodes it. It used to run once per
// ingest batch and once per query register/unregister, which on a large windowed
// list meant hundreds of whole-store serializations per scroll, for a snapshot
// the browser could never read back. Snapshots are now opt-in and checkpointed.
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

  it('never snapshots on ingest or register/unregister by default', async () => {
    const { service, persistence } = makeService();
    await service.init();
    const processor = processorInstances[0];

    service.registerQueryPlan(plan);
    service.ingest('thing', 'CREATE', 'thing:a', { id: 'thing:a' });
    service.ingestMany([
      { table: 'thing', op: 'CREATE', id: 'thing:b', record: { id: 'thing:b' } },
      { table: 'thing', op: 'CREATE', id: 'thing:c', record: { id: 'thing:c' } },
    ]);
    service.unregisterQueryPlan('q1');

    expect(processor.save_state).not.toHaveBeenCalled();
    expect(persistence.set).not.toHaveBeenCalled();
  });

  it('still never snapshots inline when persistence is opted in', async () => {
    const { service, persistence } = makeService();
    service.configureCircuitPersistence(true, 30_000);
    await service.init();
    const processor = processorInstances[0];

    service.registerQueryPlan(plan);
    service.ingest('thing', 'CREATE', 'thing:a', { id: 'thing:a' });
    service.unregisterQueryPlan('q1');

    // Marked dirty, but the write waits for the checkpoint interval.
    expect(processor.save_state).not.toHaveBeenCalled();
    expect(persistence.set).not.toHaveBeenCalled();
    service.stopCheckpoints();
  });

  it('writes at most one snapshot per checkpoint tick', async () => {
    vi.useFakeTimers();
    try {
      const { service } = makeService();
      service.configureCircuitPersistence(true, 1000);
      await service.init();
      const processor = processorInstances[0];

      for (let i = 0; i < 50; i++) {
        service.ingest('thing', 'CREATE', `thing:${i}`, { id: `thing:${i}` });
      }
      await vi.advanceTimersByTimeAsync(1000);
      expect(processor.save_state).toHaveBeenCalledTimes(1);

      // Idle interval: nothing changed, so nothing is serialized.
      await vi.advanceTimersByTimeAsync(1000);
      expect(processor.save_state).toHaveBeenCalledTimes(1);

      service.stopCheckpoints();
    } finally {
      vi.useRealTimers();
    }
  });

  it('restores a snapshot the shipped persistence clients actually return', async () => {
    const { service, persistence } = makeService();
    // Both clients round-trip `save_state`'s output as a bare string. The old
    // shape check looked for a raw SurrealDB result, so it never matched and the
    // browser never restored anything it wrote.
    persistence.get.mockResolvedValue('state-bytes' as any);
    service.configureCircuitPersistence(true);
    await service.init();
    expect(processorInstances[0].load_state).toHaveBeenCalledWith('state-bytes');
  });

  it('does not restore a snapshot when persistence is off', async () => {
    const { service, persistence } = makeService();
    persistence.get.mockResolvedValue('state-bytes' as any);
    await service.init();
    expect(processorInstances[0].load_state).not.toHaveBeenCalled();
  });
});
