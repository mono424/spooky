/**
 * Opening the worker's SQLite handle: the OPFS SAHPool VFS when durable
 * storage was asked for, an in-memory DB only as a last resort. Extracted from
 * `sqlite-worker.ts` (which imports the wasm module at module scope and so
 * can't be loaded in a unit test) to keep the retry/fallback policy testable
 * off-worker, the same split as `sqlite-select.ts`.
 *
 * Why retry: SAHPool holds an EXCLUSIVE sync access handle on every file in
 * its pool, so only one client per pool name can have it open. A second tab of
 * the same app therefore fails init, and `installOpfsSAHPoolVfs` CACHES that
 * rejection per VFS name, so a later call only gets a real second chance when
 * it passes `forceReinitIfPreviouslyFailed`. Retrying with that flag turns the
 * common "the other tab is still closing" race into a success instead of a
 * permanent in-memory session.
 *
 * Why the noise: `:memory:` holds the whole dataset in RAM (the
 * OOM-on-wasm-heavy-pages failure mode the OPFS store exists to avoid) and
 * drops every local write on reload. Host apps run pino at their own level,
 * some at `fatal`, so the fallback ALSO writes to `console.error` from inside
 * the worker, and the reason travels back to the engine as `opfsError` for the
 * app to surface.
 */

/** The DB surface the worker uses (a `sqlite3.oo1.DB` or an `OpfsSAHPoolDb`). */
export interface SqliteDbHandle {
  exec: (opts: { sql: string; bind?: unknown[]; rowMode?: string; returnValue?: string }) => unknown;
  close: () => void;
}

/** The slice of the OpfsSAHPoolUtil the worker needs for teardown. */
export interface SqlitePoolHandle {
  /** Unregisters the VFS and releases every sync access handle, leaving the
   *  files intact, so another worker can open the pool without waiting for
   *  this worker to be garbage collected. Throws while files are open. */
  pauseVfs?: () => unknown;
}

export interface OpenDbResult {
  db: SqliteDbHandle;
  /** True only when the handle is backed by OPFS and survives a reload. */
  persisted: boolean;
  /** Why persistence failed. Set only when OPFS was requested and fell back. */
  opfsError?: string;
  /** Pool util, present only for an OPFS-backed handle. */
  pool?: SqlitePoolHandle;
}

export interface OpenDbOptions {
  /** Total OPFS init attempts, including the first. Default 3. */
  maxAttempts?: number;
  /** Delay before each retry; the last entry repeats. Default [250, 500]. */
  backoffMs?: number[];
  /**
   * Throw (`opfs-unavailable: <reason>`) instead of falling back to memory.
   * Used by shared-tabs leader promotion: a silently-in-memory LEADER would
   * put every tab's data in RAM, so promotion prefers failing the election
   * (the broker retries, possibly on another tab) over degrading. The broker
   * grants an explicit memory fallback only after repeated failed cycles.
   */
  disallowMemoryFallback?: boolean;
  /** Injectable for tests. */
  sleep?: (ms: number) => Promise<void>;
}

const DEFAULT_MAX_ATTEMPTS = 3;
/** Bounded on purpose: this runs on the boot path, before the first query. */
const DEFAULT_BACKOFF_MS = [250, 500];

/**
 * Leader-promotion profile (~5.8s worst case). A dead leader's sync access
 * handles release when the browser garbage-collects its worker, typically
 * well under a second but not synchronously with the Web Lock release the
 * election observed, so promotion retries longer than a cold boot.
 */
export const PROMOTION_OPEN_OPTIONS: Pick<OpenDbOptions, 'maxAttempts' | 'backoffMs'> = {
  maxAttempts: 10,
  backoffMs: [50, 100, 200, 400, 800, 1000],
};

/** Failures no retry can fix: the APIs aren't there at all (insecure context,
 *  or a browser without sync access handles). Fall back immediately. */
const UNRETRYABLE = ['Missing required OPFS APIs'];

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Keep the DOMException name (e.g. `NoModificationAllowedError` for a pool
 *  locked by another tab): it is the most diagnostic part of the failure. */
function errMessage(e: unknown): string {
  if (e instanceof Error) return e.name && e.name !== 'Error' ? `${e.name}: ${e.message}` : e.message;
  return String(e);
}

function fallbackToMemory(
  sqlite3: any,
  dbName: string,
  reason: string,
  attempts: number
): OpenDbResult {
  const tried = attempts > 0 ? ` after ${attempts} attempt${attempts === 1 ? '' : 's'}` : '';
  // Deliberately console, not the logger: host apps configure pino's level (some
  // run `fatal`), and losing durability must never be filtered into silence.
  // oxlint-disable-next-line no-console
  console.error(
    `[sp00ky] OPFS persistence unavailable for "${dbName}"${tried}: ${reason}. The local SQLite ` +
      'cache is running IN MEMORY, which keeps the whole dataset in RAM and loses every local ' +
      'write on reload. The usual cause is another tab of this app holding the storage lock, so ' +
      'closing the other tabs and reloading restores persistence.'
  );
  return { db: new sqlite3.oo1.DB(':memory:', 'c'), persisted: false, opfsError: reason };
}

/**
 * Open `dbName`'s handle. Never throws for a storage problem: a caller that
 * asked for persistence and can't have it gets a working in-memory handle plus
 * `persisted: false` and an `opfsError` to report.
 */
export async function openDb(
  sqlite3: any,
  dbName: string,
  useOpfs: boolean,
  opts: OpenDbOptions = {}
): Promise<OpenDbResult> {
  // Memory was the configured choice (`store: 'memory'`), not a failure, so no
  // error and no noise.
  if (!useOpfs) return { db: new sqlite3.oo1.DB(':memory:', 'c'), persisted: false };

  if (!sqlite3.installOpfsSAHPoolVfs) {
    if (opts.disallowMemoryFallback) {
      throw new Error('opfs-unavailable: sqlite-wasm build has no installOpfsSAHPoolVfs');
    }
    return fallbackToMemory(sqlite3, dbName, 'sqlite-wasm build has no installOpfsSAHPoolVfs', 0);
  }

  const maxAttempts = Math.max(1, opts.maxAttempts ?? DEFAULT_MAX_ATTEMPTS);
  const backoffMs = opts.backoffMs ?? DEFAULT_BACKOFF_MS;
  const sleep = opts.sleep ?? defaultSleep;

  let lastError = 'unknown error';
  let attempts = 0;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    attempts = attempt;
    try {
      // `initialCapacity` stays at the sqlite-wasm default (6 files): one pool
      // per bucket holds a single DB plus its journals, so preallocating more
      // OPFS files would only be waste. A "SAH pool is full" error still
      // reaches the caller verbatim via `opfsError`.
      const pool = await sqlite3.installOpfsSAHPoolVfs({
        name: `sp00ky-${dbName}`,
        // The first failure is cached against the VFS name, so a retry that
        // doesn't ask for a real re-init just replays the same rejection.
        ...(attempt > 1 ? { forceReinitIfPreviouslyFailed: true } : {}),
      });
      return { db: new pool.OpfsSAHPoolDb(`/${dbName}.sqlite3`), persisted: true, pool };
    } catch (e) {
      lastError = errMessage(e);
      if (attempt === maxAttempts || UNRETRYABLE.some((m) => lastError.includes(m))) break;
      await sleep(backoffMs[Math.min(attempt - 1, backoffMs.length - 1)] ?? 0);
    }
  }
  if (opts.disallowMemoryFallback) {
    throw new Error(`opfs-unavailable: ${lastError} (after ${attempts} attempts)`);
  }
  return fallbackToMemory(sqlite3, dbName, lastError, attempts);
}
