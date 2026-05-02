import type { IngestRequest, SchedulerClient } from "./scheduler-http.js";
import { nowMs } from "../util/time.js";
import { sleep } from "../util/wait.js";

export interface ConcurrentIngestOptions {
  /** Worker pool size (max concurrent in-flight POSTs). */
  concurrency: number;
  /** Target events per second. The driver paces dispatch to hit this rate. */
  ratePerSec: number;
  /** Wall-clock seconds to run. */
  durationSec: number;
}

export interface ConcurrentIngestResult {
  /** Per-event accept latency (POST /ingest send → 200 received). */
  acceptLatenciesMs: number[];
  /** Number of events successfully accepted (HTTP 200). */
  accepted: number;
  /** Number of events that failed to be accepted (non-200 or thrown). */
  failed: number;
  /** Wall-clock duration the loop actually ran. */
  wallMs: number;
  /** Targeted rate (events/sec). */
  targetRatePerSec: number;
  /** Achieved rate (accepted/sec). */
  achievedRatePerSec: number;
}

/**
 * Drives `POST /ingest` at a target rate using a fixed-size worker pool.
 *
 * The driver dispatches one event every (1000 / ratePerSec) ms by adding it to
 * a bounded in-flight slot pool of size `concurrency`. If the pool is full, the
 * dispatch waits, so the achieved rate cannot exceed what the system can drain
 * even if `ratePerSec` is set higher.
 *
 * NOTE: this measures **accept latency** (scheduler 200 response), NOT
 * end-to-end propagation. Pair with a canary-view poller for end-to-end at
 * concurrency=1; for ramp tests at high concurrency the accept latency is the
 * meaningful per-event signal because canary polling can't distinguish
 * concurrent overlapping events.
 */
export async function runConcurrentIngest(
  client: SchedulerClient,
  events: () => IngestRequest,
  opts: ConcurrentIngestOptions,
): Promise<ConcurrentIngestResult> {
  const { concurrency, ratePerSec, durationSec } = opts;
  if (concurrency < 1) throw new Error("concurrency must be >= 1");
  if (ratePerSec <= 0) throw new Error("ratePerSec must be > 0");

  const intervalMs = 1000 / ratePerSec;
  const acceptLatenciesMs: number[] = [];
  let accepted = 0;
  let failed = 0;

  const inFlight = new Set<Promise<void>>();
  const start = nowMs();
  const deadline = start + durationSec * 1000;
  let nextDispatchMs = start;

  while (nowMs() < deadline) {
    // Backpressure: wait for a free slot if pool is full.
    if (inFlight.size >= concurrency) {
      await Promise.race(inFlight);
      continue;
    }

    // Pace toward the target rate.
    const now = nowMs();
    if (now < nextDispatchMs) {
      const wait = Math.min(nextDispatchMs - now, deadline - now);
      if (wait > 0) await sleep(wait);
      if (nowMs() >= deadline) break;
    }
    nextDispatchMs += intervalMs;

    const ev = events();
    const t0 = nowMs();
    const p = client
      .ingest(ev)
      .then(() => {
        acceptLatenciesMs.push(nowMs() - t0);
        accepted++;
      })
      .catch(() => {
        failed++;
      })
      .finally(() => {
        inFlight.delete(p);
      });
    inFlight.add(p);
  }

  // Drain the in-flight pool.
  await Promise.allSettled(inFlight);
  const wallMs = nowMs() - start;

  return {
    acceptLatenciesMs,
    accepted,
    failed,
    wallMs,
    targetRatePerSec: ratePerSec,
    achievedRatePerSec: wallMs > 0 ? (accepted / wallMs) * 1000 : 0,
  };
}
