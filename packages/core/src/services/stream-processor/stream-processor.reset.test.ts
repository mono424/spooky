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
  const service = new StreamProcessorService({} as any, {} as any, persistence as any, silentLogger);
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
    await service.init();
    service.setStateKeySuffix('u1');
    await service.saveState();
    expect(persistence.set).toHaveBeenCalledWith('_00_stream_processor_state:u1', 'state-bytes');
  });

  it('drops a saveState that raced a reset (old circuit never persists into the new key)', async () => {
    const { service, persistence } = makeService();
    await service.init();
    // The snapshot is taken, then a reset lands before the persist step —
    // simulated by bumping the generation from inside save_state().
    processorInstances[0].save_state.mockImplementation(() => {
      void service.reset();
      return 'stale-circuit';
    });
    await service.saveState();
    expect(persistence.set).not.toHaveBeenCalled();
  });
});
