import { describe, it, expect, beforeEach } from 'vitest';
import { StreamProcessorService } from './index';
import type { StreamUpdate, StreamUpdateReceiver } from './index';
import type { WasmProcessor, WasmStreamUpdate } from './wasm-types';

/**
 * Tests for the batch-coalescing window added to StreamProcessorService.
 *
 * A batched ingest (e.g. sync fetching N missing records) used to emit one
 * stream update per record, making the UI render row-by-row. beginBatch/
 * endBatch collapse those into a single coalesced update per affected query.
 */

function makeLogger(): any {
  const noop = () => {};
  const logger: any = {
    debug: noop,
    info: noop,
    warn: noop,
    error: noop,
    trace: noop,
  };
  logger.child = () => logger;
  return logger;
}

/** Records every dispatched update so we can count notifications. */
class RecordingReceiver implements StreamUpdateReceiver {
  public received: StreamUpdate[] = [];
  onStreamUpdate(update: StreamUpdate): void {
    this.received.push(update);
  }
}

/**
 * Build a service with a stubbed WASM processor. The processor accumulates
 * ingested ids and returns the *full* materialized array on every call (as the
 * real WASM does), so coalescing's last-write-wins must yield the final array.
 */
function makeService(queryHashesFor: (id: string) => string[]) {
  const svc = new StreamProcessorService(
    {} as any,
    {} as any,
    { get: async () => undefined, set: async () => {} } as any,
    makeLogger()
  );

  const ingestedByQuery = new Map<string, Array<[string, number]>>();
  const mockProcessor: Partial<WasmProcessor> = {
    ingest: (_table, _op, id, record: any): WasmStreamUpdate[] => {
      const version = record?._00_rv ?? 1;
      const updates: WasmStreamUpdate[] = [];
      for (const queryHash of queryHashesFor(id)) {
        const arr = ingestedByQuery.get(queryHash) ?? [];
        arr.push([id, version]);
        ingestedByQuery.set(queryHash, arr);
        updates.push({ query_id: queryHash, result_data: [...arr] });
      }
      return updates;
    },
  };
  // processor is private and normally set by init() via WASM; inject the stub.
  (svc as any).processor = mockProcessor;
  return svc;
}

describe('StreamProcessor batch coalescing', () => {
  let receiver: RecordingReceiver;

  beforeEach(() => {
    receiver = new RecordingReceiver();
  });

  it('emits one update per record when NOT batching (baseline)', () => {
    const svc = makeService(() => ['q1']);
    svc.addReceiver(receiver);

    svc.ingest('user', 'CREATE', 'user:1', { id: 'user:1', _00_rv: 1 });
    svc.ingest('user', 'CREATE', 'user:2', { id: 'user:2', _00_rv: 1 });
    svc.ingest('user', 'CREATE', 'user:3', { id: 'user:3', _00_rv: 1 });

    expect(receiver.received).toHaveLength(3);
  });

  it('coalesces a batch into a single update per query with the final array', () => {
    const svc = makeService(() => ['q1']);
    svc.addReceiver(receiver);

    svc.beginBatch();
    try {
      svc.ingest('user', 'CREATE', 'user:1', { id: 'user:1', _00_rv: 1 });
      svc.ingest('user', 'CREATE', 'user:2', { id: 'user:2', _00_rv: 1 });
      svc.ingest('user', 'CREATE', 'user:3', { id: 'user:3', _00_rv: 1 });
    } finally {
      svc.endBatch();
    }

    // Nothing dispatched until endBatch, then exactly one coalesced update.
    expect(receiver.received).toHaveLength(1);
    const update = receiver.received[0];
    expect(update.queryHash).toBe('q1');
    // Last-write-wins carries the full materialized array (all 3 records).
    expect(update.localArray).toHaveLength(3);
    // Coalesced updates take DataModule's immediate (non-debounced) path.
    expect(update.op).toBe('CREATE');
    expect(typeof update.materializationTimeMs).toBe('number');
  });

  it('coalesces independently per affected query', () => {
    // user:1 -> q1, user:2 -> q1 & q2, user:3 -> q2
    const svc = makeService((id) =>
      id === 'user:1' ? ['q1'] : id === 'user:2' ? ['q1', 'q2'] : ['q2']
    );
    svc.addReceiver(receiver);

    svc.beginBatch();
    try {
      svc.ingest('user', 'CREATE', 'user:1', { id: 'user:1', _00_rv: 1 });
      svc.ingest('user', 'CREATE', 'user:2', { id: 'user:2', _00_rv: 1 });
      svc.ingest('user', 'CREATE', 'user:3', { id: 'user:3', _00_rv: 1 });
    } finally {
      svc.endBatch();
    }

    expect(receiver.received).toHaveLength(2);
    const byHash = Object.fromEntries(receiver.received.map((u) => [u.queryHash, u]));
    expect(Object.keys(byHash).sort()).toEqual(['q1', 'q2']);
  });

  it('endBatch is safe with no buffered updates, and beginBatch is idempotent', () => {
    const svc = makeService(() => ['q1']);
    svc.addReceiver(receiver);

    svc.beginBatch();
    svc.beginBatch(); // no-op, must not reset/duplicate
    svc.endBatch();

    expect(receiver.received).toHaveLength(0);
    // A subsequent non-batched ingest dispatches normally (window fully closed).
    svc.ingest('user', 'CREATE', 'user:1', { id: 'user:1', _00_rv: 1 });
    expect(receiver.received).toHaveLength(1);
  });
});
