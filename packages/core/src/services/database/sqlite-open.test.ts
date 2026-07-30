import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { openDb } from './sqlite-open';

class FakeDb {
  constructor(public arg: unknown) {}
  exec() {
    return [];
  }
  close() {}
}

/** Minimal stand-in for the initialized sqlite-wasm module. `install` is the
 *  `installOpfsSAHPoolVfs` behavior under test; omit it to model a build that
 *  lacks the SAHPool VFS entirely. */
function makeSqlite3(install?: (opts: any) => Promise<unknown>) {
  const sqlite3: any = { oo1: { DB: FakeDb } };
  if (install) sqlite3.installOpfsSAHPoolVfs = vi.fn(install);
  return sqlite3;
}

const pool = { OpfsSAHPoolDb: FakeDb };
/** What a pool locked by another tab of the app actually throws. */
function lockedError(): Error {
  const e = new Error('Access Handles cannot be acquired');
  e.name = 'NoModificationAllowedError';
  return e;
}

const noSleep = () => Promise.resolve();

let errSpy: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => {
  errSpy.mockRestore();
});

// The bare `catch {}` this replaces turned every OPFS failure into a silent
// full-RAM database: no log, no reason, no way for the app to know its writes
// die on reload. Each case below pins one half of the fix: keep trying when
// retrying can plausibly work, and when it can't, say so loudly and hand the
// reason back.
describe('openDb', () => {
  it('opens the OPFS pool on the first attempt', async () => {
    const sqlite3 = makeSqlite3(async () => pool);
    const res = await openDb(sqlite3, 'user:abc', true, { sleep: noSleep });

    expect(res.persisted).toBe(true);
    expect(res.opfsError).toBeUndefined();
    expect(sqlite3.installOpfsSAHPoolVfs).toHaveBeenCalledTimes(1);
    // The pool is named per bucket, and the first try must NOT force a re-init
    // (that would throw away a pool another attempt is legitimately using).
    expect(sqlite3.installOpfsSAHPoolVfs.mock.calls[0][0]).toEqual({ name: 'sp00ky-user:abc' });
    expect(errSpy).not.toHaveBeenCalled();
  });

  // The tab-closing race: the old tab still holds the sync access handles when
  // the new one boots. Without a retry that tab is stuck in RAM for its whole
  // lifetime, even though the lock frees milliseconds later.
  it('retries a locked pool and succeeds, forcing re-init after the first failure', async () => {
    let calls = 0;
    const sqlite3 = makeSqlite3(async () => {
      if (++calls < 3) throw lockedError();
      return pool;
    });
    const res = await openDb(sqlite3, 'main', true, { sleep: noSleep });

    expect(res.persisted).toBe(true);
    expect(res.opfsError).toBeUndefined();
    expect(sqlite3.installOpfsSAHPoolVfs).toHaveBeenCalledTimes(3);
    // sqlite-wasm caches the first rejection against the VFS name, so retries
    // that don't ask for a real re-init just replay it.
    const [first, second, third] = sqlite3.installOpfsSAHPoolVfs.mock.calls.map((c: any[]) => c[0]);
    expect(first.forceReinitIfPreviouslyFailed).toBeUndefined();
    expect(second.forceReinitIfPreviouslyFailed).toBe(true);
    expect(third.forceReinitIfPreviouslyFailed).toBe(true);
    expect(errSpy).not.toHaveBeenCalled();
  });

  it('falls back loudly after exhausting the retries, keeping the reason', async () => {
    const sqlite3 = makeSqlite3(async () => {
      throw lockedError();
    });
    const res = await openDb(sqlite3, 'main', true, { sleep: noSleep });

    expect(res.persisted).toBe(false);
    // The DOMException name is the diagnostic part: it names the lock.
    expect(res.opfsError).toContain('NoModificationAllowedError');
    expect(sqlite3.installOpfsSAHPoolVfs).toHaveBeenCalledTimes(3);
    expect(errSpy).toHaveBeenCalledTimes(1);
    expect(String(errSpy.mock.calls[0][0])).toContain('IN MEMORY');
    // Still a usable handle: losing durability must not break the app.
    expect(res.db).toBeDefined();
  });

  // An insecure context has no sync access handles at all, so retrying just
  // adds boot latency to a foregone conclusion.
  it('does not retry when the OPFS APIs are missing entirely', async () => {
    const sqlite3 = makeSqlite3(async () => {
      throw new Error('Missing required OPFS APIs.');
    });
    const res = await openDb(sqlite3, 'main', true, { sleep: noSleep });

    expect(res.persisted).toBe(false);
    expect(res.opfsError).toContain('Missing required OPFS APIs');
    expect(sqlite3.installOpfsSAHPoolVfs).toHaveBeenCalledTimes(1);
    expect(errSpy).toHaveBeenCalledTimes(1);
  });

  it('reports a build without the SAHPool VFS without calling anything', async () => {
    const sqlite3 = makeSqlite3();
    const res = await openDb(sqlite3, 'main', true, { sleep: noSleep });

    expect(res.persisted).toBe(false);
    expect(res.opfsError).toContain('installOpfsSAHPoolVfs');
    expect(errSpy).toHaveBeenCalledTimes(1);
  });

  // `store: 'memory'` is a configuration choice, not a degradation, so it must
  // stay silent and carry no error for the UI to warn about.
  it('opens in memory quietly when persistence was not requested', async () => {
    const sqlite3 = makeSqlite3(async () => pool);
    const res = await openDb(sqlite3, 'main', false, { sleep: noSleep });

    expect(res.persisted).toBe(false);
    expect(res.opfsError).toBeUndefined();
    expect(sqlite3.installOpfsSAHPoolVfs).not.toHaveBeenCalled();
    expect(errSpy).not.toHaveBeenCalled();
  });

  it('honors maxAttempts and waits the configured backoff between tries', async () => {
    const slept: number[] = [];
    const sqlite3 = makeSqlite3(async () => {
      throw lockedError();
    });
    const res = await openDb(sqlite3, 'main', true, {
      maxAttempts: 4,
      backoffMs: [10, 20],
      sleep: async (ms) => {
        slept.push(ms);
      },
    });

    expect(res.persisted).toBe(false);
    expect(sqlite3.installOpfsSAHPoolVfs).toHaveBeenCalledTimes(4);
    // One sleep per gap (never after the last attempt), last delay repeating.
    expect(slept).toEqual([10, 20, 20]);
  });
});
