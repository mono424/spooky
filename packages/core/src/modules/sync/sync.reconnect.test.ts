import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Sp00kySync } from './sync';

// The reconnect handler re-registers active queries and re-issues the ref LIVE.
// It must fire on BOTH drop paths, and the distinction is the whole bug:
//
//   - recovered drop:  error -> reconnecting -> connected   (no `disconnected`!)
//   - gave-up drop:    error -> disconnected -> (supervisor) -> connected
//
// Watching only `disconnected` therefore misses every *successful* reconnect —
// the common case — and leaves the dead server-side LIVE in place with the
// list_ref poll silently the only sync path.

/** Minimal stand-in for the SDK's event publisher. */
function makeClient() {
  const listeners = new Map<string, Array<(...a: any[]) => void>>();
  return {
    subscribe(event: string, cb: (...a: any[]) => void) {
      const arr = listeners.get(event) ?? [];
      arr.push(cb);
      listeners.set(event, arr);
      return () => {
        /* noop */
      };
    },
    emit(event: string, ...args: any[]) {
      for (const cb of listeners.get(event) ?? []) cb(...args);
    },
    liveOf: async () => ({ subscribe: () => () => {} }),
  };
}

function makeSync(hashes: string[], userId: string | null = 'user:alice') {
  const logger: any = {
    child: () => logger,
    debug: () => {},
    info: () => {},
    warn: () => {},
    error: () => {},
    trace: () => {},
  };
  const client = makeClient();
  const status = { value: 'connected' as string };
  const remote: any = {
    query: vi.fn().mockResolvedValue(['live-uuid']),
    getClient: () => client,
    getStatus: () => status.value,
  };
  const dataModule: any = {
    getActiveQueryHashes: () => hashes,
    getCurrentUserId: () => userId,
  };

  const sync = new Sp00kySync({} as any, remote, {} as any, dataModule, {} as any, logger);

  // `subscribeToReconnect` is normally wired from init(), which would also start
  // timers and load the outbox. Call it directly instead.
  (sync as any).currentUserId = userId;
  (sync as any).subscribeToReconnect();

  const enqueued: any[] = [];
  (sync as any).scheduler = { enqueueDownEvent: (e: any) => enqueued.push(e) };

  return { sync, client, remote, enqueued, status };
}

/** Let the `connected` handler's un-awaited `restartRefLiveQuery()` chain run. */
const flush = () => new Promise((r) => setTimeout(r, 0));

describe('sync reconnect re-subscription', () => {
  beforeEach(() => vi.clearAllMocks());

  it('re-registers queries and restarts LIVE on a recovered drop (reconnecting -> connected)', async () => {
    const { client, remote, enqueued } = makeSync(['h1', 'h2']);

    // The SDK's own reconnect: no `disconnected` is ever published.
    client.emit('reconnecting');
    client.emit('connected');
    await flush();

    expect(enqueued).toEqual([
      { type: 'register', payload: { hash: 'h1' } },
      { type: 'register', payload: { hash: 'h2' } },
    ]);
    const sqls = (remote.query as any).mock.calls.map((c: any[]) => c[0]);
    expect(sqls.some((s: string) => s.startsWith('LIVE SELECT'))).toBe(true);
  });

  it('re-registers queries and restarts LIVE after the SDK gives up (disconnected -> connected)', async () => {
    const { client, remote, enqueued } = makeSync(['h1']);

    client.emit('disconnected');
    client.emit('connected');
    await flush();

    expect(enqueued).toEqual([{ type: 'register', payload: { hash: 'h1' } }]);
    const sqls = (remote.query as any).mock.calls.map((c: any[]) => c[0]);
    expect(sqls.some((s: string) => s.startsWith('LIVE SELECT'))).toBe(true);
  });

  it('does nothing on the initial connect', async () => {
    const { client, remote, enqueued } = makeSync(['h1']);

    client.emit('connected');
    await flush();

    expect(enqueued).toEqual([]);
    expect(remote.query).not.toHaveBeenCalled();
  });

  it('only reacts once per drop', async () => {
    const { client, enqueued } = makeSync(['h1']);

    client.emit('reconnecting');
    client.emit('connected');
    await flush();
    // A spurious second `connected` (e.g. a re-published event) must not
    // trigger a second refetch storm.
    client.emit('connected');
    await flush();

    expect(enqueued).toHaveLength(1);
  });

  it("does not KILL the dead session's LIVE uuid on reconnect", async () => {
    const { sync, client, remote } = makeSync(['h1']);
    // A LIVE was running before the drop. Its uuid belongs to the old
    // WebSocket session, so KILLing it on the new socket is pointless work
    // queued ahead of the restart that actually matters.
    const unsub = vi.fn();
    (sync as any).currentLiveQueryUuid = 'stale-uuid';
    (sync as any).liveQueryUnsubscribe = unsub;

    client.emit('reconnecting');
    expect(unsub).toHaveBeenCalled();
    expect((sync as any).currentLiveQueryUuid).toBeNull();

    client.emit('connected');
    await flush();

    const sqls = (remote.query as any).mock.calls.map((c: any[]) => c[0]);
    expect(sqls.some((q: string) => q.includes('KILL'))).toBe(false);
    expect(sqls.some((q: string) => q.startsWith('LIVE SELECT'))).toBe(true);
  });
});
