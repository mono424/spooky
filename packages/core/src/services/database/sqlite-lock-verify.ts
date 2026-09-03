/**
 * "Do we still hold the OPFS lock?" - asked by the SQLite worker after a gap
 * in its ticks long enough that the tab may have been frozen and the broker
 * may have handed the lock to another tab.
 *
 * Bounded. `navigator.locks.query()` has no deadline of its own, and every op
 * on the worker's chain awaits this verification: one query that never
 * resolves stopped every reply to every client for good. On timeout we resolve
 * WITHOUT fencing - the steal callback registered on the lock request remains
 * the backstop that fences a genuinely stolen lock - and say so.
 */
export interface LockQueryLike {
  query(): Promise<{ held?: { name?: string }[] }>;
}

export const LOCK_VERIFY_TIMEOUT_MS = 2_000;

export async function verifyLockStillHeld(
  locks: LockQueryLike | null | undefined,
  name: string | undefined,
  fence: (reason: string) => Promise<void> | void,
  timeoutMs = LOCK_VERIFY_TIMEOUT_MS,
  warn: (msg: string) => void = () => {}
): Promise<void> {
  if (!locks || !name || typeof locks.query !== 'function') return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const state = await Promise.race([
      locks.query(),
      new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), timeoutMs);
      }),
    ]);
    if (state === null) {
      warn(`lock verification did not answer within ${timeoutMs}ms; trusting the steal callback`);
      return;
    }
    const stillHeld = (state.held ?? []).some((l) => l.name === name);
    if (!stillHeld) await fence('lock missing after suspected freeze');
  } catch {
    /* query unavailable: fall back to the steal callback */
  } finally {
    if (timer) clearTimeout(timer);
  }
}
