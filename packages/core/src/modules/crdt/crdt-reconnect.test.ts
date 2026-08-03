import { describe, it, expect, vi } from 'vitest';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import { CrdtManager } from './index';
import type { LocalStore, RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';

// A LIVE subscription lives and dies with its WebSocket session, and
// `ensureTableSubscription` is memoized on `liveByTable`. Without a reconnect
// hook, the first dropped socket kills CRDT realtime for good: the map still
// holds a uuid for a subscription the server has forgotten, so every later
// `open()` short-circuits and no LIVE is ever re-issued.

const noop = () => {};

/** A promise plus its resolver, so a test can hold an RPC open on demand. */
function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function silentLogger(): Logger {
  const fake: any = {};
  fake.info = noop;
  fake.warn = noop;
  fake.debug = noop;
  fake.error = noop;
  fake.trace = noop;
  fake.child = () => fake;
  return fake as Logger;
}

function singleTableSchema(): SchemaStructure {
  return {
    tables: [
      {
        name: 'thread',
        columns: { body: { type: 'string', optional: false, crdt: 'text' } },
        primaryKey: ['id'],
      },
    ],
    relationships: [],
    backends: {},
  };
}

function makeRemote() {
  const handlers = new Map<string, Array<(...a: any[]) => void>>();
  const remote = {
    query: vi.fn().mockImplementation(async (sql: string) => {
      if (sql.includes('LIVE SELECT')) return ['live-uuid'];
      if (sql.includes('SELECT * FROM ONLY')) return [null];
      return [];
    }),
    getClient: () =>
      ({ liveOf: async () => ({ subscribe: () => () => {} }) }) as any,
    getStatus: () => 'connected' as const,
    subscribeConnection: (event: string, cb: (...a: any[]) => void) => {
      handlers.set(event, [...(handlers.get(event) ?? []), cb]);
      return () => {
        handlers.set(event, (handlers.get(event) ?? []).filter((h) => h !== cb));
      };
    },
  } satisfies Partial<RemoteDatabaseService> as unknown as RemoteDatabaseService;
  const emit = (event: string) => {
    for (const cb of Array.from(handlers.get(event) ?? [])) cb();
  };
  return { remote, emit };
}

function makeLocal(): LocalStore {
  return {
    query: vi.fn().mockResolvedValue([null]),
  } satisfies Partial<LocalStore> as unknown as LocalStore;
}

const liveSelects = (remote: RemoteDatabaseService) =>
  (remote.query as ReturnType<typeof vi.fn>).mock.calls
    .map((c) => c[0] as string)
    .filter((s) => s.includes('LIVE SELECT'));

describe('CrdtManager reconnect', () => {
  it('re-issues the table LIVE after a recovered drop', async () => {
    const { remote, emit } = makeRemote();
    const manager = new CrdtManager(singleTableSchema(), makeLocal(), remote, silentLogger());
    manager.setSessionId('s1');

    await manager.open('thread', 'thread:abc', 'body');
    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(1));

    // The SDK's own reconnect path: `reconnecting` then `connected`, never
    // `disconnected`.
    emit('reconnecting');
    emit('connected');

    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(2));
    // No KILL: that uuid belonged to a session that no longer exists.
    const sqls = (remote.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
    expect(sqls.some((s) => s.includes('KILL'))).toBe(false);
  });

  it('re-issues the table LIVE after the SDK gives up', async () => {
    const { remote, emit } = makeRemote();
    const manager = new CrdtManager(singleTableSchema(), makeLocal(), remote, silentLogger());
    manager.setSessionId('s1');

    await manager.open('thread', 'thread:abc', 'body');
    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(1));

    emit('disconnected');
    emit('connected');

    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(2));
  });

  it('does not resurrect a LIVE for a table whose fields were all closed', async () => {
    const { remote, emit } = makeRemote();
    const manager = new CrdtManager(singleTableSchema(), makeLocal(), remote, silentLogger());
    manager.setSessionId('s1');

    await manager.open('thread', 'thread:abc', 'body');
    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(1));

    // A drop can outlive the editor that needed the feed.
    emit('reconnecting');
    manager.close('thread', 'thread:abc', 'body');
    emit('connected');

    await new Promise((r) => setTimeout(r, 10));
    expect(liveSelects(remote)).toHaveLength(1);
  });

  it('discards and re-registers a LIVE that was mid-flight when the socket dropped', async () => {
    const handlers = new Map<string, Array<() => void>>();
    const firstLive = deferred<void>();
    let liveCalls = 0;
    const remote = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('LIVE SELECT')) {
          liveCalls++;
          // Hold the FIRST registration open so the drop lands mid-flight.
          if (liveCalls === 1) {
            await firstLive.promise;
            return ['dead-session-uuid'];
          }
          return ['fresh-uuid'];
        }
        return [null];
      }),
      getClient: () => ({ liveOf: async () => ({ subscribe: () => () => {} }) }) as any,
      getStatus: () => 'connected' as const,
      subscribeConnection: (event: string, cb: () => void) => {
        handlers.set(event, [...(handlers.get(event) ?? []), cb]);
        return () => {};
      },
    } satisfies Partial<RemoteDatabaseService> as unknown as RemoteDatabaseService;
    const emit = (event: string) => {
      for (const cb of Array.from(handlers.get(event) ?? [])) cb();
    };

    const manager = new CrdtManager(singleTableSchema(), makeLocal(), remote, silentLogger());
    manager.setSessionId('s1');
    await manager.open('thread', 'thread:abc', 'body');
    await vi.waitFor(() => expect(liveCalls).toBe(1));

    // Drop + recover while the first registration is still awaiting its uuid.
    emit('reconnecting');
    emit('connected');
    firstLive.resolve();

    // The doomed uuid must never be recorded — if it were, `liveByTable` would
    // short-circuit every future restart and CRDT realtime would be dead for
    // good. A fresh registration must take its place.
    await vi.waitFor(() => expect(liveCalls).toBe(2));
    expect((manager as any).liveByTable.get('thread')).toBe('fresh-uuid');
  });

  it('stops reacting to transport events after dispose', async () => {
    const { remote, emit } = makeRemote();
    const manager = new CrdtManager(singleTableSchema(), makeLocal(), remote, silentLogger());
    manager.setSessionId('s1');

    await manager.open('thread', 'thread:abc', 'body');
    await vi.waitFor(() => expect(liveSelects(remote)).toHaveLength(1));

    manager.dispose();
    emit('reconnecting');
    emit('connected');

    await new Promise((r) => setTimeout(r, 10));
    expect(liveSelects(remote)).toHaveLength(1);
  });
});
