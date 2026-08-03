import { describe, it, expect, vi } from 'vitest';
import { LoroDoc } from 'loro-crdt';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import { CrdtManager } from './index';
import type { LocalStore, RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';

// Regression test for the offline-reload formatting-loss class of bug.
//
// Repro pattern: app uses `store: 'memory'` for the local SurrealDB (or
// any scenario where the local row exists without a CRDT snapshot —
// fresh device, post-sync-down with stale meta, …). On reload, the
// editor mounts but stays empty until some unrelated edit fires the
// parent LIVE feed. From the user's POV, formatting is gone.
//
// Fix: when `CrdtManager.open` finds no local snapshot, it must fetch
// the parent row from remote (the snapshot lives inline on the row in
// the consolidated design) and dispatch its CRDT field. This test pins
// that behavior across both shapes — `@crdt`-only fields where the
// snapshot IS the field value, and `@crdt @cursor` fields where the
// snapshot lives at `<field>.state`.

function silentLogger(): Logger {
  const noop = () => {};
  const fake: any = {};
  fake.info = noop;
  fake.warn = noop;
  fake.debug = noop;
  fake.error = noop;
  fake.trace = noop;
  fake.child = () => fake;
  return fake as Logger;
}

function singleTableSchema(opts: { cursor?: boolean } = {}): SchemaStructure {
  return {
    tables: [
      {
        name: 'thread',
        columns: {
          body: {
            type: 'string',
            optional: false,
            crdt: 'text',
            ...(opts.cursor ? { cursor: true } : {}),
          },
        },
        primaryKey: ['id'],
      },
    ],
    relationships: [],
    backends: {},
  };
}

function buildSnapshot(text: string): Uint8Array {
  const doc = new LoroDoc();
  doc.getText('text').insert(0, text);
  return doc.export({ mode: 'snapshot' });
}

function makeRemote(handlers: {
  selectRow?: () => unknown;
}): RemoteDatabaseService {
  const remote = {
    query: vi.fn().mockImplementation(async (sql: string) => {
      if (sql.includes('LIVE SELECT')) return ['live-uuid'];
      if (sql.includes('SELECT * FROM ONLY')) {
        return [handlers.selectRow ? handlers.selectRow() : null];
      }
      return [];
    }),
    getClient: () =>
      ({
        liveOf: async () => ({
          subscribe: () => () => {},
        }),
      }) as any,
    // CrdtManager watches transport events so it can re-issue table LIVEs after
    // a reconnect. These tests never drop the socket, so a no-op is enough.
    getStatus: () => 'connected' as const,
    subscribeConnection: vi.fn().mockReturnValue(() => {}),
  } satisfies Partial<RemoteDatabaseService> as unknown as RemoteDatabaseService;
  return remote;
}

describe('CrdtManager.open hydration', () => {
  it('@crdt-only: fetches the parent row from remote when local is empty', async () => {
    const remoteSnapshot = buildSnapshot('hello world');

    const local = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('SELECT VALUE body FROM ONLY')) return [null];
        return [];
      }),
    } satisfies Partial<LocalStore> as unknown as LocalStore;

    const remote = makeRemote({
      selectRow: () => ({ id: 'thread:abc', body: remoteSnapshot }),
    });

    const manager = new CrdtManager(singleTableSchema(), local, remote, silentLogger());
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:abc', 'body');

    await vi.waitFor(() => {
      expect(field.getDoc().getText('text').toString()).toBe('hello world');
    });

    const sqls = (remote.query as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => c[0] as string
    );
    expect(sqls.some((s) => s.includes('SELECT * FROM ONLY'))).toBe(true);
  });

  it('@crdt @cursor: extracts the snapshot from the object shape', async () => {
    const remoteSnapshot = buildSnapshot('object shape');

    const local = {
      query: vi.fn().mockImplementation(async () => [null]),
    } satisfies Partial<LocalStore> as unknown as LocalStore;

    // Cursor-enabled field stores `{ state, cursors }`. The remote row
    // returns this shape; the manager must drill into `.state` for the
    // snapshot bytes.
    const remote = makeRemote({
      selectRow: () => ({
        id: 'thread:abc',
        body: { state: remoteSnapshot, cursors: {} },
      }),
    });

    const manager = new CrdtManager(
      singleTableSchema({ cursor: true }),
      local,
      remote,
      silentLogger()
    );
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:abc', 'body');

    await vi.waitFor(() => {
      expect(field.getDoc().getText('text').toString()).toBe('object shape');
    });
  });

  it('skips the remote fetch when a local snapshot already exists', async () => {
    const localSnapshot = buildSnapshot('cached');

    const local = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('SELECT VALUE body FROM ONLY')) return [localSnapshot];
        return [];
      }),
    } satisfies Partial<LocalStore> as unknown as LocalStore;

    const remote = makeRemote({});

    const manager = new CrdtManager(singleTableSchema(), local, remote, silentLogger());
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:def', 'body');

    expect(field.getDoc().getText('text').toString()).toBe('cached');

    // Give any stray async work a tick to land before asserting absence.
    await new Promise((r) => setTimeout(r, 20));

    const sqls = (remote.query as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => c[0] as string
    );
    expect(sqls.some((s) => s.includes('SELECT * FROM ONLY'))).toBe(false);
  });

  // Regression: SurrealDB returns bytes nested inside a FLEXIBLE object as a
  // plain `number[]` from the local WASM engine (e.g. `state: [108, 111, 114,
  // 111, ...]` — the literal LoroDoc magic header bytes). The cursor-enabled
  // extractSnapshot path used to only accept Uint8Array and would silently
  // return undefined for the array form, leaving every editor empty and
  // killing realtime updates for `@crdt @cursor` fields.
  it('@crdt @cursor: accepts number[] as the state shape (local WASM transport)', async () => {
    const remoteSnapshot = buildSnapshot('from local wasm');
    const remoteAsArray = Array.from(remoteSnapshot) as unknown as number[];

    const local = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('SELECT VALUE body FROM ONLY')) {
          return [{ state: remoteAsArray, cursors: {} }];
        }
        return [];
      }),
    } satisfies Partial<LocalStore> as unknown as LocalStore;

    const remote = makeRemote({});

    const manager = new CrdtManager(
      singleTableSchema({ cursor: true }),
      local,
      remote,
      silentLogger()
    );
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:wasm', 'body');

    expect(field.getDoc().getText('text').toString()).toBe('from local wasm');
  });
});
