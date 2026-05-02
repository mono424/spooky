import type { SchedulerClient } from "../drivers/scheduler-http.js";
import { nowMs } from "../util/time.js";
import { sleep } from "../util/wait.js";

export interface CatchUpSample {
  /** ms since `start`. */
  t: number;
  /** scheduler.lag (latest_seq − snapshot_seq). */
  lag: number;
  /** scheduler.snapshot_seq at this poll. */
  snapshotSeq: number;
  /** scheduler.latest_seq at this poll. */
  latestSeq: number;
  /** scheduler.pending_events. */
  pendingEvents: number;
  /** scheduler.ready_ssps. */
  readySsps: number;
}

export interface CatchUpResult {
  /** Time series of /metrics samples from `start` until lag reached 0 (or timeout). */
  samples: CatchUpSample[];
  /** Total wall ms from start of catch-up to lag == 0. -1 if timed out. */
  catchUpMs: number;
  /** Whether `lag` actually reached 0 within `timeoutMs`. */
  caughtUp: boolean;
  /** Initial backlog (events to drain). */
  initialBacklog: number;
  /** Replay throughput (events/sec) during the catch-up window. */
  replayRatePerSec: number;
}

/**
 * Polls scheduler `/metrics` from `start` until `lag` reaches 0 (or we hit
 * `timeoutMs`). Used by the backlog suite after restarting the SSP to
 * measure how fast the scheduler-buffered events drain into the SSP.
 *
 * NOTE: `lag` here is the scheduler's *replica*-relative lag, not view
 * propagation lag. They're conceptually distinct in sp00ky. For this suite
 * we report both: time until `lag == 0` AND the time until a probe view's
 * cache_size reflects the expected post-replay state. Caller decides which
 * to consider authoritative.
 */
export async function awaitCatchUp(
  client: SchedulerClient,
  opts: { intervalMs?: number; timeoutMs?: number } = {},
): Promise<CatchUpResult> {
  const intervalMs = opts.intervalMs ?? 250;
  const timeoutMs = opts.timeoutMs ?? 60_000;
  const samples: CatchUpSample[] = [];
  const start = nowMs();
  let initialBacklog = -1;
  let caughtUpAt: number | null = null;

  while (nowMs() - start < timeoutMs) {
    try {
      const m = await client.metrics();
      const sample: CatchUpSample = {
        t: nowMs() - start,
        lag: m.scheduler.lag,
        snapshotSeq: m.scheduler.snapshot_seq,
        latestSeq: m.scheduler.latest_seq,
        pendingEvents: m.scheduler.pending_events,
        readySsps: m.scheduler.ready_ssps,
      };
      samples.push(sample);
      if (initialBacklog < 0) initialBacklog = sample.lag;
      if (sample.lag === 0 && sample.readySsps >= 1) {
        caughtUpAt = sample.t;
        break;
      }
    } catch {
      /* transient, keep polling */
    }
    await sleep(intervalMs);
  }

  const catchUpMs = caughtUpAt ?? -1;
  const replayRatePerSec =
    catchUpMs > 0 && initialBacklog > 0 ? (initialBacklog / catchUpMs) * 1000 : 0;

  return {
    samples,
    catchUpMs,
    caughtUp: caughtUpAt !== null,
    initialBacklog: Math.max(0, initialBacklog),
    replayRatePerSec,
  };
}
