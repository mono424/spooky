/**
 * Web Lock helpers for the leader TAB side. (The worker lock lives inside
 * sqlite-worker.ts; the broker's probe/steal helpers live in the broker
 * worker. This module is only what a tab needs to claim leadership.)
 */

export interface LeaderLockHandle {
  /** Release the lock on purpose (demotion, shutdown). Idempotent. */
  release(): void;
  /** Fires exactly once if the lock is taken away rather than released. */
  onLost(cb: () => void): void;
}

function lockManager(): LockManager | null {
  const nav = (globalThis as { navigator?: { locks?: LockManager } }).navigator;
  return nav?.locks && typeof nav.locks.request === 'function' ? nav.locks : null;
}

/**
 * Acquire `name` exclusively. `steal: true` when the broker determined the
 * previous holder is a frozen zombie. Resolves null when the lock is held by
 * someone else and stealing was not requested.
 */
export function acquireLeaderTabLock(
  name: string,
  opts: { steal: boolean }
): Promise<LeaderLockHandle | null> {
  const locks = lockManager();
  // No Web Locks means shared-tabs support detection failed earlier; treat as
  // granted so tests without the API can still exercise the flow.
  if (!locks) {
    return Promise.resolve({ release() {}, onLost() {} });
  }
  return new Promise((resolve) => {
    let lostCb: (() => void) | null = null;
    let releasedIntentionally = false;
    let granted = false;
    let release: (() => void) | null = null;
    locks
      .request(
        name,
        opts.steal ? { mode: 'exclusive', steal: true } : { mode: 'exclusive', ifAvailable: true },
        (lock) => {
          if (!lock) {
            resolve(null);
            return;
          }
          granted = true;
          resolve({
            release() {
              releasedIntentionally = true;
              release?.();
            },
            onLost(cb) {
              lostCb = cb;
            },
          });
          return new Promise<void>((r) => {
            release = r;
          });
        }
      )
      .then(
        () => {
          if (granted && !releasedIntentionally) lostCb?.();
        },
        () => {
          // A steal against us settles the request with an AbortError-like
          // rejection in some engines; treat exactly like a loss.
          if (granted && !releasedIntentionally) lostCb?.();
          else if (!granted) resolve(null);
        }
      );
  });
}
