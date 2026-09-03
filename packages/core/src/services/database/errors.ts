/**
 * A local-store operation that did not answer within its deadline.
 *
 * The local write path (`db.create` / `db.update` / `db.delete`, every local
 * query behind them) used to have no deadline anywhere: the SQLite worker
 * transport parks a call until the worker replies, the surrealdb engine's
 * query chain waits on the previous link, and `withRetry` retries without a
 * clock. One op that never settled (a worker starved behind a long select, a
 * lock verification awaiting `navigator.locks.query()` forever) left the
 * caller's promise pending for the tab's lifetime - a chat composer that never
 * re-enabled, a call that never got past "Connecting".
 *
 * The message says "timed out" on purpose: `classifySyncError` keys off it and
 * treats the failure as transient (re-queue), never as an application error
 * that rolls the mutation back. `retryable: false` keeps `withRetry` from
 * spinning on it: the op is still running in the engine, retrying queues a
 * second copy behind it.
 */
export class LocalOpTimeoutError extends Error {
  override readonly name = 'LocalOpTimeoutError';
  readonly retryable = false;
  readonly op: string;
  readonly timeoutMs: number;

  constructor(op: string, timeoutMs: number) {
    super(`Local database operation timed out after ${timeoutMs}ms (${op})`);
    this.op = op;
    this.timeoutMs = timeoutMs;
  }
}

/** Default deadline for one local-store operation. Generous: a cold 4k-row
 *  select on a throttled tab is seconds, not tens of seconds. */
export const DEFAULT_LOCAL_OP_TIMEOUT_MS = 30_000;
