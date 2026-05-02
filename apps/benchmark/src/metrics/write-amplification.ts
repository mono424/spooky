import type { SspClient } from "../drivers/ssp-http.js";
import { nowMs } from "../util/time.js";
import { sleep } from "../util/wait.js";

export interface AmplificationSample {
  /** Cache size BEFORE the input event. */
  before: number;
  /** Cache size AFTER the cascade settled. */
  after: number;
  /** Output records emitted (after - before). */
  delta: number;
  /** Time from input dispatch to first observation that the hash changed AND cache_size differs. */
  settleMs: number;
  /** Hash before. */
  hashBefore: string;
  /** Hash after. */
  hashAfter: string;
}

/**
 * Capture write-amplification for a single triggering event.
 *
 * Reads canary state, calls `dispatch()` (which sends the event), then polls
 * `/debug/view/:id` until BOTH the hash changes AND the cache size has
 * stabilized for `--stable-window-ms` (default 200ms). Returns the cache_size
 * delta as the amplification factor.
 *
 * The dual condition (hash change + cache size stability) handles cascades:
 * a single thread update fans out to K joined rows, all applied in one
 * circuit step, so the hash flips once but cache_size jumps from B to B+K
 * atomically. The stability window catches the case where the SSP applies
 * the cascade across multiple ticks.
 */
export async function measureAmplification(
  ssp: SspClient,
  canaryId: string,
  dispatch: () => Promise<void>,
  opts: { timeoutMs?: number; stableWindowMs?: number } = {},
): Promise<AmplificationSample> {
  const timeoutMs = opts.timeoutMs ?? 10_000;
  const stableWindowMs = opts.stableWindowMs ?? 200;

  const initial = await ssp.getDebugView(canaryId);
  if (!initial) throw new Error(`canary ${canaryId} not registered`);
  const before = initial.cache_size;
  const hashBefore = initial.last_hash;

  const start = nowMs();
  await dispatch();

  let hashChangedAt: number | null = null;
  let lastSize = before;
  let lastChangeMs = -1;
  let lastHash = hashBefore;

  while (nowMs() - start < timeoutMs) {
    const v = await ssp.getDebugView(canaryId);
    if (!v) {
      await sleep(1);
      continue;
    }
    if (v.last_hash !== hashBefore) {
      if (hashChangedAt === null) {
        hashChangedAt = nowMs();
        lastChangeMs = hashChangedAt;
      }
      if (v.cache_size !== lastSize) {
        lastSize = v.cache_size;
        lastChangeMs = nowMs();
      }
      lastHash = v.last_hash;
      // Stable for the window after the hash first changed → cascade settled.
      if (nowMs() - lastChangeMs >= stableWindowMs) {
        return {
          before,
          after: lastSize,
          delta: lastSize - before,
          settleMs: hashChangedAt! - start,
          hashBefore,
          hashAfter: lastHash,
        };
      }
    }
    await new Promise((r) => setImmediate(r));
  }

  return {
    before,
    after: lastSize,
    delta: lastSize - before,
    settleMs: timeoutMs,
    hashBefore,
    hashAfter: lastHash,
  };
}
