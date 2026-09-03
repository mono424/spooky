/// <reference lib="webworker" />
/**
 * Dedicated Web Worker that owns the SQLite-WASM handle. All DB access is
 * funnelled through here (the main thread never touches the wasm module), which
 * is also what the OPFS VFS requires — file access must happen off the main
 * thread. Persistence uses the **OPFS SAHPool VFS**: durable, and (unlike the
 * classic OPFS VFS) it does NOT require COOP/COEP cross-origin isolation
 * headers, so host apps embedding the client need no server changes. When OPFS
 * is unavailable it retries, then falls back to an in-memory DB and REPORTS the
 * loss of durability (see `sqlite-open.ts`) instead of degrading silently.
 *
 * Message protocol (request/response keyed by `id`; replies go to the channel
 * the request arrived on):
 *   { id, type: 'open',   payload: { dbName, useOpfs, workerLockName? } }
 *        -> { id, ok, persisted, opfsError? }
 *   { id, type: 'exec',   payload: { sql, bind } }  -> { id, ok, rows }
 *   { id, type: 'run',    payload: { sql, bind } }  -> { id, ok }
 *   { id, type: 'batch',  payload: [{ sql, bind }] } (atomic BEGIN/COMMIT)
 *   { id, type: 'select', payload: { plan, params } } -> { id, ok, rows, relationFetches }
 *   { id, type: 'close' }
 *   { id, type: 'shutdown' }                     (owner only: close + pauseVfs + self.close)
 *   { id, type: 'relock', payload: { workerLockName } }               (owner only)
 *   { id, type: 'add-client', payload: { clientId } } + ev.ports[0]   (owner only)
 *   { id, type: 'remove-client', payload: { clientId } }              (owner only)
 *
 * Multi-client (shared-tabs mode): the OWNER (the leader tab's engine) speaks
 * on the worker's own channel and controls the lifecycle. Follower tabs get a
 * MessagePort each (`add-client`) and may only issue data ops
 * (exec/run/batch/select). Every op, from any channel, runs through ONE op
 * chain so an async `select` from one client can never interleave with another
 * client's `batch` at the VFS layer.
 *
 * Leader fencing (shared-tabs mode): `open` may carry `workerLockName`, a Web
 * Lock this worker acquires BEFORE opening the pool and holds for the DB's
 * lifetime. The name is unique per leadership, so the lock being gone has
 * exactly one meaning: the broker stole it because this tab was presumed dead
 * (usually frozen). When that happens the worker FENCES itself: closes the DB,
 * releases the OPFS sync access handles (pauseVfs), tells the owner, and
 * refuses every further op. A frozen-then-thawed ex-leader therefore can never
 * write the durable file after a new leader took over. The thaw gate below
 * closes the wake-up ordering race where a queued write could run before the
 * lock-loss callback fires.
 *
 * `select` executes a whole QueryPlan — table creation, base select, the full
 * `.related()` tree (shared `resolveRelations`), and JSON row parsing — in ONE
 * round-trip, returning structured-clone row objects. This is the first-load
 * hot path; the per-statement ops remain for the write/shim paths.
 */
import sqlite3InitModule from '@sqlite.org/sqlite-wasm';
import { verifyLockStillHeld as verifyLockStillHeldBounded } from './sqlite-lock-verify';
import {
  openDb,
  type OpenDbOptions,
  type SqliteDbHandle,
  type SqlitePoolHandle,
} from './sqlite-open';
import { executeSelect, type SelectDb } from './sqlite-select';

interface Stmt {
  sql: string;
  bind?: unknown[];
}

interface PostTarget {
  postMessage: (msg: unknown) => void;
}

let db: SqliteDbHandle | null = null;
let pool: SqlitePoolHandle | null = null;

// ==================== leader fencing (worker web lock) ====================

/** Once fenced, every op is refused and the DB stays closed. Terminal. */
let fenced = false;

interface HeldLock {
  name: string;
  /** Release on purpose; the closure-local flag keeps the steal handler from
   *  mistaking it for a takeover. Idempotent. */
  release: () => void;
}

/** The per-leadership lock currently held (null = no fencing active). A
 *  relock (same-leader re-promotion after a broker restart) briefly holds two
 *  locks: the new one is installed here first, then the old one's own release
 *  is called, so there is no window where neither is held. */
let currentLock: HeldLock | null = null;

function workerLocks(): LockManager | null {
  const nav = (globalThis as { navigator?: { locks?: LockManager } }).navigator;
  return nav?.locks && typeof nav.locks.request === 'function' ? nav.locks : null;
}

/**
 * Acquire `name` exclusively without waiting. Resolves with a handle when
 * granted, null when someone else holds it. If the request settles while the
 * holder still believes it owns the lock, the broker stole it: fence.
 */
function acquireWorkerLock(name: string): Promise<HeldLock | null> {
  const locks = workerLocks();
  // No Web Locks (non-browser test env): no fencing, behave as before.
  if (!locks) return Promise.resolve({ name, release() {} });
  return new Promise<HeldLock | null>((resolve) => {
    let granted = false;
    let intentional = false;
    let releaseGrant: (() => void) | null = null;
    locks
      .request(name, { mode: 'exclusive', ifAvailable: true }, (lock) => {
        if (!lock) {
          resolve(null);
          return;
        }
        granted = true;
        resolve({
          name,
          release() {
            intentional = true;
            releaseGrant?.();
          },
        });
        return new Promise<void>((release) => {
          releaseGrant = release;
        });
      })
      .then(
        () => {
          if (granted && !intentional) void fence('worker lock stolen');
        },
        () => {
          if (granted && !intentional) void fence('worker lock request failed');
        }
      );
  });
}

function releaseHeldLock(): void {
  currentLock?.release();
  currentLock = null;
}

/**
 * Terminal teardown after leadership loss. Closing the DB first makes
 * pauseVfs legal (it throws while files are open); pausing releases the sync
 * access handles so the NEW leader's open succeeds without waiting for this
 * worker to be garbage collected.
 */
async function fence(reason: string): Promise<void> {
  if (fenced) return;
  fenced = true;
  // Deliberately console, not a logger: this must be visible in any host app
  // regardless of its configured log level.
  // oxlint-disable-next-line no-console
  console.error(
    `[sp00ky] sqlite worker fenced (${reason}): leadership was taken over, ` +
      'closing the database and refusing further ops.'
  );
  try {
    db?.close();
  } catch {
    /* already closed */
  }
  db = null;
  try {
    pool?.pauseVfs?.();
  } catch {
    /* pool already torn down or files still open; new leader retries anyway */
  }
  pool = null;
  try {
    (self as unknown as Worker).postMessage({ type: 'lock-lost', reason });
  } catch {
    /* owner gone */
  }
  releaseHeldLock();
  self.close();
}

// ==================== thaw gate ====================

/**
 * A frozen tab's worker resumes with its message queue intact, and a queued
 * write could run BEFORE the lock-steal callback fires. Both the 1s interval
 * and every op call `noteTick()`; whichever runs first after a long gap kicks
 * off a lock verification, and ops await it before touching the DB. The gap
 * threshold must stay BELOW the broker's pong timeout: a freeze shorter than
 * the threshold cannot have triggered a steal yet, so skipping the check for
 * small gaps is safe.
 */
const FREEZE_SUSPECT_MS = 10_000;
let lastTickAt = performance.now();
let thawVerification: Promise<void> | null = null;

function noteTick(): void {
  const now = performance.now();
  const gap = now - lastTickAt;
  lastTickAt = now;
  if (gap > FREEZE_SUSPECT_MS && currentLock && !fenced && !thawVerification) {
    thawVerification = verifyLockStillHeld().finally(() => {
      thawVerification = null;
    });
  }
}
setInterval(noteTick, 1000);

// Bounded (see sqlite-lock-verify.ts): every op on the chain awaits this, and
// a `locks.query()` that never answered stopped every reply to every client.
function verifyLockStillHeld(): Promise<void> {
  return verifyLockStillHeldBounded(
    workerLocks() as any,
    currentLock?.name,
    (reason) => fence(reason),
    undefined,
    (msg) => console.warn(`[sqlite-worker] ${msg}`)
  );
}

// ==================== db ops ====================

async function open(
  dbName: string,
  useOpfs: boolean,
  systemTables: readonly string[] = [],
  workerLockName?: string,
  openOptions?: OpenDbOptions
): Promise<{ persisted: boolean; opfsError?: string }> {
  if (workerLockName) {
    const handle = await acquireWorkerLock(workerLockName);
    if (!handle) throw new Error('worker-lock-unavailable');
    releaseHeldLock();
    currentLock = handle;
  }
  const sqlite3: any = await sqlite3InitModule();
  // Retry/fallback policy (and the loud report when persistence is lost) lives
  // in `sqlite-open.ts` so it can be unit tested off-worker.
  const result = await openDb(sqlite3, dbName, useOpfs, openOptions ?? {});
  db = result.db;
  pool = result.pool ?? null;
  // Physically create the internal `_00_*` tables the client reads before any
  // write (DEFINE is a noop on this engine, so the migrator can't). Prevents
  // "no such table: _00_query" on a fresh bucket right after signup.
  for (const t of systemTables) {
    db!.exec({ sql: `CREATE TABLE IF NOT EXISTS "${t}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)` });
  }
  // Retry (rather than instantly failing with SQLITE_BUSY=5) if a lock is held.
  // Combined with the single-flight op chain, overlap is avoided.
  // `cache_size` is negated → KiB (here 32 MiB) so SQLite's page cache can't
  // grow unbounded and starve a wasm-heavy renderer.
  try {
    db!.exec({ sql: 'PRAGMA busy_timeout = 5000; PRAGMA cache_size = -32000;' });
  } catch {
    /* pragma best-effort */
  }
  return { persisted: result.persisted, opfsError: result.opfsError };
}

function closeDb(): void {
  db?.close();
  db = null;
  pool = null;
  selectDb.knownTables.clear();
  // The lock is scoped to (bucket, leadership); a same-worker bucket switch
  // closes then reopens under a NEW name, so the old one must go now.
  releaseHeldLock();
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
 *  plan executor). `knownTables` is cleared on open/close, which covers both
 *  the solo fresh-worker path and the shared-mode same-worker bucket switch. */
const selectDb: SelectDb = {
  exec: (sql, bind) => exec(sql, bind) as { data: string }[],
  run,
  knownTables: new Set<string>(),
};

// ==================== dispatch ====================

/** Follower client ports, keyed by the clientId the owner assigned. */
const clients = new Map<string, MessagePort>();

/** Data ops a follower client port may issue; lifecycle stays owner-only. */
const CLIENT_OPS = new Set(['exec', 'run', 'batch', 'select']);

/**
 * ONE chain for every op from every channel. The handlers for `open` and
 * `select` are async; without this, a follower's select could interleave with
 * the owner's batch mid-transaction at the VFS layer. (Solo mode kept this
 * invariant on the main thread via the engine's opQueue; with multiple client
 * ports only the worker can.)
 */
let opChain: Promise<void> = Promise.resolve();

async function handle(type: string, payload: any, source: 'owner' | 'client'): Promise<unknown> {
  if (fenced) throw new Error('sqlite: fenced, leadership was taken over');
  if (source === 'client' && !CLIENT_OPS.has(type)) {
    throw new Error(`sqlite worker: op ${type} is not allowed on a client port`);
  }
  switch (type) {
    case 'open':
      selectDb.knownTables.clear();
      {
        const result = await open(
          payload.dbName,
          payload.useOpfs,
          payload.systemTables,
          payload.workerLockName,
          payload.openOptions
        );
        // The freshly seeded system tables exist — record them so the select
        // path doesn't redundantly re-issue CREATE TABLE for each.
        for (const t of (payload.systemTables ?? []) as string[]) selectDb.knownTables.add(t);
        return result;
      }
    case 'relock': {
      // Same-leader re-promotion after a broker restart: the DB stays open,
      // only the per-leadership lock name rolls forward. Acquire the new lock
      // BEFORE releasing the old one so there is no unfenced window.
      const handle = await acquireWorkerLock(payload.workerLockName);
      if (!handle) throw new Error('worker-lock-unavailable');
      releaseHeldLock();
      currentLock = handle;
      return {};
    }
    case 'select':
      return executeSelect(payload.plan, payload.params ?? {}, selectDb);
    case 'exec':
      return { rows: exec(payload.sql, payload.bind) };
    case 'run':
      run(payload.sql, payload.bind);
      return {};
    case 'batch':
      batch(payload as Stmt[]);
      return {};
    case 'close':
      closeDb();
      return {};
    case 'shutdown':
      // Graceful pagehide path: pausing the VFS releases the sync access
      // handles NOW instead of whenever this worker gets garbage collected,
      // so the next leader's open does not race the browser's GC.
      try {
        db?.close();
      } catch {
        /* ignore */
      }
      db = null;
      try {
        pool?.pauseVfs?.();
      } catch {
        /* ignore */
      }
      pool = null;
      releaseHeldLock();
      queueMicrotask(() => self.close());
      return {};
    case 'remove-client': {
      const port = clients.get(payload.clientId);
      port?.close();
      clients.delete(payload.clientId);
      return {};
    }
    default:
      throw new Error(`sqlite worker: unknown message ${type}`);
  }
}

function dispatch(target: PostTarget, data: any, source: 'owner' | 'client'): void {
  const { id, type, payload } = data ?? {};
  noteTick();
  const t0 = performance.now();
  const runOp = async () => {
    try {
      if (thawVerification) await thawVerification;
      const result = await handle(type, payload, source);
      // `wt` (worker time) lets the main thread split a round-trip into actual
      // DB work vs postMessage/queue overhead (see `__sqliteStats`).
      target.postMessage({ id, ok: true, wt: performance.now() - t0, ...(result as object) });
    } catch (err) {
      target.postMessage({ id, ok: false, error: err instanceof Error ? err.message : String(err) });
    }
  };
  opChain = opChain.then(runOp, runOp);
}

self.onmessage = (ev: MessageEvent) => {
  const data = ev.data ?? {};
  // `add-client` is handled inline (not on the op chain): it only registers a
  // port and must not wait behind a long select, or the follower's first ops
  // (already queued on that port by the time the ack arrives) would deadlock
  // the attach handshake in the engine.
  if (data.type === 'add-client') {
    const port = ev.ports?.[0];
    const clientId = data.payload?.clientId as string | undefined;
    if (!port || !clientId) {
      (self as unknown as Worker).postMessage({
        id: data.id,
        ok: false,
        error: 'sqlite worker: add-client needs a clientId and a transferred port',
      });
      return;
    }
    clients.get(clientId)?.close();
    clients.set(clientId, port);
    port.onmessage = (pe: MessageEvent) => dispatch(port, pe.data, 'client');
    port.start?.();
    (self as unknown as Worker).postMessage({ id: data.id, ok: true, wt: 0 });
    return;
  }
  dispatch(self as unknown as Worker, data, 'owner');
};
