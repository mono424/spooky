/// <reference lib="webworker" />
/**
 * Dedicated Web Worker that owns the SQLite-WASM handle. All DB access is
 * funnelled through here (the main thread never touches the wasm module), which
 * is also what the OPFS VFS requires — file access must happen off the main
 * thread. Persistence uses the **OPFS SAHPool VFS**: durable, and (unlike the
 * classic OPFS VFS) it does NOT require COOP/COEP cross-origin isolation
 * headers, so host apps embedding the client need no server changes. Falls back
 * to an in-memory DB when OPFS is unavailable.
 *
 * Message protocol (request/response keyed by `id`):
 *   { id, type: 'open',   payload: { dbName, useOpfs } }
 *   { id, type: 'exec',   payload: { sql, bind } }  -> { id, ok, rows }
 *   { id, type: 'run',    payload: { sql, bind } }  -> { id, ok }
 *   { id, type: 'batch',  payload: [{ sql, bind }] } (atomic BEGIN/COMMIT)
 *   { id, type: 'select', payload: { plan, params } } -> { id, ok, rows, relationFetches }
 *   { id, type: 'close' }
 *
 * `select` executes a whole QueryPlan — table creation, base select, the full
 * `.related()` tree (shared `resolveRelations`), and JSON row parsing — in ONE
 * round-trip, returning structured-clone row objects. This is the first-load
 * hot path; the per-statement ops remain for the write/shim paths.
 */
import sqlite3InitModule from '@sqlite.org/sqlite-wasm';
import { executeSelect, type SelectDb } from './sqlite-select';

interface Stmt {
  sql: string;
  bind?: unknown[];
}

let db: {
  exec: (opts: { sql: string; bind?: unknown[]; rowMode?: string; returnValue?: string }) => unknown;
  close: () => void;
} | null = null;

async function open(
  dbName: string,
  useOpfs: boolean,
  systemTables: readonly string[] = []
): Promise<{ persisted: boolean }> {
  const sqlite3: any = await sqlite3InitModule();
  let persisted = false;
  if (useOpfs && sqlite3.installOpfsSAHPoolVfs) {
    try {
      const pool = await sqlite3.installOpfsSAHPoolVfs({ name: `sp00ky-${dbName}` });
      db = new pool.OpfsSAHPoolDb(`/${dbName}.sqlite3`);
      persisted = true;
    } catch {
      // fall through to in-memory
    }
  }
  if (!db) db = new sqlite3.oo1.DB(':memory:', 'c');
  // Physically create the internal `_00_*` tables the client reads before any
  // write (DEFINE is a noop on this engine, so the migrator can't). Prevents
  // "no such table: _00_query" on a fresh bucket right after signup.
  for (const t of systemTables) {
    db!.exec({ sql: `CREATE TABLE IF NOT EXISTS "${t}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)` });
  }
  // Retry (rather than instantly failing with SQLITE_BUSY=5) if a lock is held.
  // Combined with the engine's single-flight op queue, overlap is avoided.
  // `cache_size` is negated → KiB (here 32 MiB) so SQLite's page cache can't
  // grow unbounded and starve a wasm-heavy renderer.
  try {
    db!.exec({ sql: 'PRAGMA busy_timeout = 5000; PRAGMA cache_size = -32000;' });
  } catch {
    /* pragma best-effort */
  }
  return { persisted };
}

function exec(sql: string, bind?: unknown[]): unknown[] {
  if (!db) throw new Error('sqlite: DB not open');
  return db.exec({ sql, bind, rowMode: 'object', returnValue: 'resultRows' }) as unknown[];
}

function run(sql: string, bind?: unknown[]): void {
  if (!db) throw new Error('sqlite: DB not open');
  db.exec({ sql, bind });
}

function batch(stmts: Stmt[]): void {
  if (!db) throw new Error('sqlite: DB not open');
  db.exec({ sql: 'BEGIN' });
  try {
    for (const s of stmts) db.exec({ sql: s.sql, bind: s.bind });
    db.exec({ sql: 'COMMIT' });
  } catch (e) {
    try {
      db.exec({ sql: 'ROLLBACK' });
    } catch {
      /* ignore */
    }
    throw e;
  }
}

// ==================== worker-side plan execution ('select') ====================

/** DB handle for `executeSelect` (see `sqlite-select.ts`, the unit-testable
 *  plan executor). `knownTables` is cleared on open/close — a bucket switch
 *  spawns a fresh worker anyway. */
const selectDb: SelectDb = {
  exec: (sql, bind) => exec(sql, bind) as { data: string }[],
  run,
  knownTables: new Set<string>(),
};

self.onmessage = async (ev: MessageEvent) => {
  const { id, type, payload } = ev.data ?? {};
  // `wt` (worker time) lets the main thread split a round-trip into actual DB
  // work vs postMessage/queue overhead (see `__sqliteStats`).
  const t0 = performance.now();
  try {
    let result: unknown;
    switch (type) {
      case 'open':
        selectDb.knownTables.clear();
        result = await open(payload.dbName, payload.useOpfs, payload.systemTables);
        // The freshly seeded system tables exist — record them so the select
        // path doesn't redundantly re-issue CREATE TABLE for each.
        for (const t of (payload.systemTables ?? []) as string[]) selectDb.knownTables.add(t);
        break;
      case 'select':
        result = await executeSelect(payload.plan, payload.params ?? {}, selectDb);
        break;
      case 'exec':
        result = { rows: exec(payload.sql, payload.bind) };
        break;
      case 'run':
        run(payload.sql, payload.bind);
        result = {};
        break;
      case 'batch':
        batch(payload as Stmt[]);
        result = {};
        break;
      case 'close':
        db?.close();
        db = null;
        selectDb.knownTables.clear();
        result = {};
        break;
      default:
        throw new Error(`sqlite worker: unknown message ${type}`);
    }
    (self as unknown as Worker).postMessage({
      id,
      ok: true,
      wt: performance.now() - t0,
      ...(result as object),
    });
  } catch (err) {
    (self as unknown as Worker).postMessage({
      id,
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    });
  }
};
