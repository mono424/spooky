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
 *   { id, type: 'open',  payload: { dbName, useOpfs } }
 *   { id, type: 'exec',  payload: { sql, bind } }  -> { id, ok, rows }
 *   { id, type: 'run',   payload: { sql, bind } }  -> { id, ok }
 *   { id, type: 'batch', payload: [{ sql, bind }] } (atomic BEGIN/COMMIT)
 *   { id, type: 'close' }
 */
import sqlite3InitModule from '@sqlite.org/sqlite-wasm';

interface Stmt {
  sql: string;
  bind?: unknown[];
}

let db: {
  exec: (opts: { sql: string; bind?: unknown[]; rowMode?: string; returnValue?: string }) => unknown;
  close: () => void;
} | null = null;

async function open(dbName: string, useOpfs: boolean): Promise<{ persisted: boolean }> {
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

self.onmessage = async (ev: MessageEvent) => {
  const { id, type, payload } = ev.data ?? {};
  try {
    let result: unknown;
    switch (type) {
      case 'open':
        result = await open(payload.dbName, payload.useOpfs);
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
        result = {};
        break;
      default:
        throw new Error(`sqlite worker: unknown message ${type}`);
    }
    (self as unknown as Worker).postMessage({ id, ok: true, ...(result as object) });
  } catch (err) {
    (self as unknown as Worker).postMessage({
      id,
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    });
  }
};
