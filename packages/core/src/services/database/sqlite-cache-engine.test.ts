import { describe, it, expect } from 'vitest';
import { RecordId } from 'surrealdb';
import { pureWriteOpResult, SqliteCacheEngine } from './sqlite-cache-engine';
import { translateSurql } from './surql-translate';
import type { SqlOp } from './surql-translate';
import { surql } from '../../utils/surql';

function makeLogger(): any {
  const noop = () => {};
  const l: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  l.child = () => l;
  return l;
}

/**
 * A fake Worker that models the real SQLite worker's open/closed lifecycle:
 * `open` opens the DB, `close` closes it, and `exec`/`run` REJECT with
 * "sqlite: DB not open" when the DB isn't currently open — exactly the failure
 * the fix targets. Records every message `type` (across worker generations) so
 * tests can also inspect dispatch order.
 */
class FakeWorker {
  onmessage: ((ev: any) => void) | null = null;
  onerror: any = null;
  onmessageerror: any = null;
  private dbOpen = false;
  constructor(private log: string[]) {}
  postMessage(msg: any) {
    this.log.push(msg.type);
    Promise.resolve().then(() => {
      let ok = true;
      let error: string | undefined;
      let rest: Record<string, unknown> = {};
      if (msg.type === 'open') {
        this.dbOpen = true;
        rest = { persisted: true };
      } else if (msg.type === 'close') {
        this.dbOpen = false;
      } else if (msg.type === 'exec') {
        if (!this.dbOpen) { ok = false; error = 'sqlite: DB not open'; }
        else rest = { rows: [] };
      } else if (msg.type === 'select') {
        if (!this.dbOpen) { ok = false; error = 'sqlite: DB not open'; }
        else rest = { rows: [], relationFetches: 0 };
      } else if (msg.type === 'run' || msg.type === 'batch') {
        if (!this.dbOpen) { ok = false; error = 'sqlite: DB not open'; }
      }
      this.onmessage?.({ data: { id: msg.id, ok, error, ...rest } });
    });
  }
  terminate() {}
}

// Regression: switchBucket must run close → reopen as a single serialized
// opQueue entry. Otherwise a read/write enqueued during an auth/bucket change
// dispatches to the just-closed worker → "sqlite: DB not open" (the crash on
// sign-in). The invariant: no `exec` may appear between a `close` and the next
// `open` in the message stream.
describe('SqliteCacheEngine.switchBucket serialization', () => {
  it('never dispatches an op to a closed DB during a bucket switch', async () => {
    const log: string[] = [];
    const engine = new SqliteCacheEngine({ namespace: 'n', database: 'd' } as any, makeLogger());
    // Replicate the real spawnWorker's onmessage wiring (resolve/reject pending).
    (engine as any).spawnWorker = () => {
      const w = new FakeWorker(log);
      w.onmessage = (ev: any) => {
        const { id, ok, error, ...rest } = ev.data ?? {};
        const p = (engine as any).pending.get(id);
        if (!p) return;
        (engine as any).pending.delete(id);
        if (ok) p.resolve(rest);
        else p.reject(new Error(error));
      };
      return w as unknown as Worker;
    };

    await engine.connect('anon');
    expect(log).toContain('open');

    // Fire a switch and a concurrent read (exactly what the sign-in query
    // re-registration does). With the old non-atomic switch the read's `exec`
    // dispatched to the closed/terminated worker and REJECTED ("sqlite: DB not
    // open" / "not connected") — the uncaught crash. The atomic switch makes the
    // read wait for the reopen and resolve against the new bucket.
    const switching = engine.switchBucket('user:abc');
    const reading = engine.getById('_00_query', 'h1');
    await expect(Promise.all([switching, reading])).resolves.toBeDefined();
    await expect(reading).resolves.toBeNull(); // missing row → null, not a throw

    // And the close→reopen ran with no op wedged between them.
    const closeIdx = log.indexOf('close');
    const openAfter = log.indexOf('open', closeIdx + 1);
    expect(openAfter).toBeGreaterThan(closeIdx);
    expect(log.slice(closeIdx + 1, openAfter)).not.toContain('exec');
    expect(engine.currentBucketId).toBe('user:abc');
  });
});

// Regression: DEFINE is a noop on this engine, so the `_00_*` internal tables
// the migrator DEFINEs are never physically created — a fresh bucket that READS
// one before any write (the sync layer selects `_00_query` at startup) threw
// "no such table: _00_query" and wedged the client on "Loading database". The
// fix seeds them inside `open`; assert the open message carries them.
describe('SqliteCacheEngine system-table seeding', () => {
  it('passes the _00_* system tables to the worker open (fresh-bucket safe)', async () => {
    const msgs: any[] = [];
    const engine = new SqliteCacheEngine({ namespace: 'n', database: 'd' } as any, makeLogger());
    (engine as any).spawnWorker = () => {
      const w: any = { onmessage: null, onerror: null, onmessageerror: null, terminate() {} };
      w.postMessage = (msg: any) => {
        msgs.push(msg);
        Promise.resolve().then(() => {
          const rest = msg.type === 'open' ? { persisted: true } : {};
          w.onmessage?.({ data: { id: msg.id, ok: true, ...rest } });
        });
      };
      // Wire pending resolution like the real spawnWorker.
      const inner = w.onmessage;
      w.onmessage = (ev: any) => {
        const { id, ok, error, ...rest } = ev.data ?? {};
        const p = (engine as any).pending.get(id);
        if (!p) return;
        (engine as any).pending.delete(id);
        if (ok) p.resolve(rest);
        else p.reject(new Error(error));
        void inner;
      };
      return w as unknown as Worker;
    };

    await engine.connect('user:fresh');
    const open = msgs.find((m) => m.type === 'open');
    expect(open?.payload?.systemTables).toContain('_00_query');
    expect(open?.payload?.systemTables).toContain('_00_pending_mutations');
  });
});

// The worker's `persisted`/`opfsError` reply used to die in a `logger.info`
// line, so a host app running pino at `fatal` (whitepawn does) could not tell a
// disk-backed store from a full-RAM one. It now lands on the engine as
// observable state the app can render.
describe('SqliteCacheEngine storage health', () => {
  /** Engine wired to a worker whose `open` replies with `openReply`. */
  function makeEngine(openReply: Record<string, unknown>, opts?: { useOpfs?: boolean }) {
    const logs: { level: string; msg: string; meta: any }[] = [];
    const logger: any = {};
    for (const level of ['debug', 'info', 'warn', 'error', 'trace']) {
      logger[level] = (meta: any, msg: string) => logs.push({ level, msg, meta });
    }
    logger.child = () => logger;

    const engine = new SqliteCacheEngine(
      { namespace: 'n', database: 'd' } as any,
      logger,
      opts ?? {}
    );
    (engine as any).spawnWorker = () => {
      const w: any = { onmessage: null, onerror: null, onmessageerror: null, terminate() {} };
      w.postMessage = (msg: any) => {
        Promise.resolve().then(() => {
          const rest = msg.type === 'open' ? openReply : {};
          const { id, ok, error, ...payload } = { id: msg.id, ok: true, error: undefined, ...rest };
          const p = (engine as any).pending.get(id);
          if (!p) return;
          (engine as any).pending.delete(id);
          if (ok) p.resolve(payload);
          else p.reject(new Error(error));
        });
      };
      return w as unknown as Worker;
    };
    return { engine, logs };
  }

  it('publishes a persistent store and logs no error', async () => {
    const { engine, logs } = makeEngine({ persisted: true });
    await engine.connect('user:abc');

    expect(engine.storageHealth).toEqual({
      status: 'persistent',
      fallback: false,
      error: undefined,
    });
    expect(logs.some((l) => l.level === 'error')).toBe(false);
  });

  it('publishes the fallback, its reason, and an error log when OPFS is lost', async () => {
    const { engine, logs } = makeEngine({ persisted: false, opfsError: 'NoModificationAllowedError: locked' });
    await engine.connect('user:abc');

    expect(engine.storageHealth).toEqual({
      status: 'memory',
      fallback: true,
      error: 'NoModificationAllowedError: locked',
    });
    const err = logs.find((l) => l.level === 'error');
    expect(err?.msg).toContain('IN MEMORY');
    expect(err?.meta.opfsError).toBe('NoModificationAllowedError: locked');
    // Inspectable from the console without any logging configured.
    expect((globalThis as any).__sqliteStats.persisted).toBe(false);
  });

  // A subscriber almost always attaches AFTER connect() (components mount
  // later), so an immediate fire is the only way it learns about a fallback.
  it('fires a late subscriber with the current snapshot', async () => {
    const { engine } = makeEngine({ persisted: false, opfsError: 'boom' });
    await engine.connect('user:abc');

    const seen: any[] = [];
    const unsub = engine.subscribeToStorageHealth((h) => seen.push(h));
    expect(seen).toEqual([{ status: 'memory', fallback: true, error: 'boom' }]);
    unsub();
  });

  // `store: 'memory'` asked for RAM, so it is not a fallback and must not warn.
  it('does not flag a configured in-memory store as a fallback', async () => {
    const { engine, logs } = makeEngine({ persisted: false }, { useOpfs: false });
    await engine.connect('user:abc');

    expect(engine.storageHealth).toEqual({ status: 'memory', fallback: false, error: undefined });
    expect(logs.some((l) => l.level === 'error')).toBe(false);
  });
});

// Storage numbers for the DevTools Storage tab: DB size via the pragmas, row
// counts on demand, and the configured-vs-effective workerSelect split. Errors
// must land in `error` (the worker may be mid bucket-switch), never throw.
describe('SqliteCacheEngine.getStorageDiagnostics', () => {
  function makeEngine(execRows: (sql: string) => unknown[]) {
    const noop = () => {};
    const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
    logger.child = () => logger;
    const engine = new SqliteCacheEngine({ namespace: 'n', database: 'd' } as any, logger);
    (engine as any).spawnWorker = () => {
      const w: any = { onmessage: null, onerror: null, onmessageerror: null, terminate() {} };
      w.postMessage = (msg: any) => {
        Promise.resolve().then(() => {
          const rest =
            msg.type === 'open'
              ? { persisted: true }
              : msg.type === 'exec'
                ? { rows: execRows(msg.payload.sql) }
                : {};
          const p = (engine as any).pending.get(msg.id);
          if (!p) return;
          (engine as any).pending.delete(msg.id);
          p.resolve(rest);
        });
      };
      return w as unknown as Worker;
    };
    return engine;
  }

  it('reports size, freelist, and per-table counts', async () => {
    const engine = makeEngine((sql) => {
      if (sql.includes('pragma_page_count')) return [{ bytes: 40960, freelist: 4096 }];
      if (sql.includes('sqlite_master')) return [{ name: '_00_query' }, { name: 'game' }];
      if (sql.includes('COUNT(*)'))
        return [
          { t: '_00_query', n: 3 },
          { t: 'game', n: 12 },
        ];
      return [];
    });
    await engine.connect('user:abc');

    const diag = await engine.getStorageDiagnostics({ tableCounts: true });
    expect(diag.engine).toBe('sqlite');
    expect(diag.bucketId).toBe('user:abc');
    expect(diag.dbSizeBytes).toBe(40960);
    expect(diag.freelistBytes).toBe(4096);
    expect(diag.tableCounts).toEqual([
      { table: '_00_query', rows: 3 },
      { table: 'game', rows: 12 },
    ]);
    // Default config: workerSelect on, never downgraded.
    expect(diag.workerSelectConfigured).toBe(true);
    expect(diag.workerSelectEffective).toBe(true);
  });

  it('skips table counts unless asked and never throws on a dead worker', async () => {
    const engine = makeEngine(() => [{ bytes: 8192, freelist: 0 }]);
    await engine.connect('anon');

    const diag = await engine.getStorageDiagnostics();
    expect(diag.tableCounts).toBeUndefined();

    // No worker at all → the failure lands in `error`, not as a throw.
    const cold = makeEngine(() => []);
    const coldDiag = await cold.getStorageDiagnostics();
    expect(coldDiag.error).toContain('not connected');
    expect(coldDiag.bucketId).toBe('anon');
  });
});

// `pureWriteOpResult` is the single source of truth for what a pure-write op
// contributes to a query's per-statement results. The batched fast path in
// `query()` and the per-op `execOp` path BOTH route through it, so a caller that
// reads a statement's output (e.g. `create()` reads `resultIndex:0` for the new
// row + its id) sees the same shape either way.
describe('pureWriteOpResult', () => {
  it('echoes the written row (with id) for an upsert — no read-back', () => {
    const op: SqlOp = {
      kind: 'upsert',
      id: 'connection:CONN_abc',
      data: { provider: 'chesscom', username: 'hikaru' },
      mode: 'replace',
    };
    expect(pureWriteOpResult(op)).toEqual({
      provider: 'chesscom',
      username: 'hikaru',
      id: 'connection:CONN_abc',
    });
  });

  it('stringifies a RecordId id via stableKey', () => {
    const op: SqlOp = {
      kind: 'upsert',
      id: new RecordId('connection', 'CONN_abc'),
      data: { provider: 'lichess' },
      mode: 'replace',
    };
    expect(pureWriteOpResult(op)).toEqual({ provider: 'lichess', id: 'connection:CONN_abc' });
  });

  it('yields [] for delete / deleteAll and null for noop', () => {
    expect(pureWriteOpResult({ kind: 'delete', id: 'game:1' })).toEqual([]);
    expect(pureWriteOpResult({ kind: 'deleteAll', table: 'game' })).toEqual([]);
    expect(pureWriteOpResult({ kind: 'noop' })).toBeNull();
  });
});

// Regression: a single `create()` compiles to an all-upsert transaction
// (createSet for the row + createMutation for the pending-mutation log) and
// extracts `resultIndex:0` for the created row. The SQLite fast path must return
// that row (with its id) at that index — returning `[]` there dropped the id and
// crashed the reconcile in `encodeRecordId` ("reading 'table'").
describe('create() tx result shaping (fast path parity)', () => {
  it('resultIndex:0 carries the created row with its id', () => {
    const rid = new RecordId('connection', 'CONN_abc');
    const mid = new RecordId('_00_pending_mutations', '1');
    const vars = {
      id: rid,
      mid,
      data_provider: 'chesscom',
      data_username: 'hikaru',
    };

    // Same statement pair DataModule.create emits.
    const sealed = surql.seal(
      surql.tx([
        surql.createSet('id', [
          { key: 'provider', variable: 'data_provider' },
          { key: 'username', variable: 'data_username' },
        ]),
        surql.createMutation('create', 'mid', 'id', 'data'),
      ]),
      { resultIndex: 0 }
    );

    const { transaction, ops } = translateSurql(sealed.sql, vars);
    expect(transaction).toBe(true);
    // Both statements are upserts → the engine takes the all-write fast path.
    expect(ops.every((o) => o.kind === 'upsert')).toBe(true);

    // Fast-path shaping: [null (BEGIN), ...one result per statement].
    const shaped = [null, ...ops.map(pureWriteOpResult)];
    const created = sealed.extract(shaped) as unknown as { id: unknown; provider: string };

    expect(created.id).toBe('connection:CONN_abc');
    expect(created.provider).toBe('chesscom');
  });
});
