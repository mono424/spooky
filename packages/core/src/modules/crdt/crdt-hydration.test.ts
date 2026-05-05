import { describe, it, expect, vi } from 'vitest';
import { LoroDoc } from 'loro-crdt';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import { CrdtManager } from './index';
import { encodeBase64 } from './crdt-field';
import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';

// Regression test for the offline-reload formatting-loss bug.
//
// Repro: app uses `store: 'memory'` for the local SurrealDB (or any
// scenario where the local row exists without a `_00_crdt` snapshot —
// fresh device, post-sync-down REPLACE, etc.). On reload, the editor
// mounts but stays empty until some unrelated edit fires the parent
// LIVE feed. From the user's POV, formatting is gone.
//
// Fix: when `CrdtManager.open` finds no local snapshot, it must fetch
// the remote `_00_crdt` row eagerly and import it into the freshly-
// created CrdtField. This test pins that behavior.

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

function singleTableSchema(): SchemaStructure {
  return {
    tables: [
      {
        name: 'thread',
        columns: {
          body: { type: 'string', optional: false, crdt: 'text' },
        },
        primaryKey: ['id'],
      },
    ],
    relationships: [],
    backends: {},
  };
}

describe('CrdtManager.open hydration', () => {
  it('fetches remote _00_crdt when local snapshot is missing', async () => {
    // Build a real Loro snapshot that the "remote" will return.
    const seedDoc = new LoroDoc();
    seedDoc.getText('text').insert(0, 'hello world');
    const remoteSnapshot = encodeBase64(seedDoc.export({ mode: 'snapshot' }));

    // Local is empty: every SELECT/UPDATE resolves harmlessly.
    // The lookup `SELECT VALUE _00_crdt[$field] FROM ONLY $id` returns
    // `[null]` to signal no persisted snapshot.
    const local = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('_00_crdt[$field]')) return [null];
        return [];
      }),
    } satisfies Partial<LocalDatabaseService> as unknown as LocalDatabaseService;

    // Remote answers two queries:
    //  1. The `LIVE SELECT * FROM thread` from `ensureTableSubscription`.
    //  2. The combined `SELECT field, state FROM _00_crdt … ; SELECT … FROM _00_cursor …`
    //     from `fetchAndDispatchMeta`.
    const remote = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('LIVE SELECT')) return ['live-uuid'];
        if (sql.includes('FROM _00_crdt')) {
          return [
            [{ field: 'body', state: remoteSnapshot }],
            [],
          ];
        }
        return [];
      }),
      getClient: () => ({
        liveOf: async () => ({
          subscribe: () => () => {},
        }),
      }),
    } satisfies Partial<RemoteDatabaseService> as unknown as RemoteDatabaseService;

    const manager = new CrdtManager(singleTableSchema(), local, remote, silentLogger());
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:abc', 'body');

    // The remote meta fetch is fire-and-forget inside `open()`, so wait
    // for the import to land. `vi.waitFor` polls until the assertion
    // passes (or times out), which is the cheapest reliable wait without
    // wiring promise plumbing into production code.
    await vi.waitFor(() => {
      expect(field.getDoc().getText('text').toString()).toBe('hello world');
    });

    const sqls = (remote.query as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => c[0] as string
    );
    expect(sqls.some((s) => s.includes('FROM _00_crdt'))).toBe(true);
  });

  it('skips the remote fetch when a local snapshot already exists', async () => {
    // Local has a real snapshot already — open() must NOT trigger the
    // remote round-trip. This is the cache-warm path that we don't want
    // to slow down with an unnecessary network hop.
    const seedDoc = new LoroDoc();
    seedDoc.getText('text').insert(0, 'cached');
    const localSnapshot = encodeBase64(seedDoc.export({ mode: 'snapshot' }));

    const local = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('_00_crdt[$field]')) return [localSnapshot];
        return [];
      }),
    } satisfies Partial<LocalDatabaseService> as unknown as LocalDatabaseService;

    const remote = {
      query: vi.fn().mockImplementation(async (sql: string) => {
        if (sql.includes('LIVE SELECT')) return ['live-uuid'];
        return [];
      }),
      getClient: () => ({
        liveOf: async () => ({
          subscribe: () => () => {},
        }),
      }),
    } satisfies Partial<RemoteDatabaseService> as unknown as RemoteDatabaseService;

    const manager = new CrdtManager(singleTableSchema(), local, remote, silentLogger());
    manager.setSessionId('session-under-test');

    const field = await manager.open('thread', 'thread:def', 'body');

    expect(field.getDoc().getText('text').toString()).toBe('cached');

    // Give any stray async work a tick to land before asserting absence.
    await new Promise((r) => setTimeout(r, 20));

    const sqls = (remote.query as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => c[0] as string
    );
    expect(sqls.some((s) => s.includes('FROM _00_crdt'))).toBe(false);
  });
});
