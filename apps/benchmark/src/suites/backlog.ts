import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { SspClient } from "../drivers/ssp-http.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { generateIngests } from "../workload/ingest.js";
import { generateQueries } from "../workload/queries.js";
import { awaitCatchUp, type CatchUpResult } from "../metrics/catch-up.js";
import { startSeededStack, safeStop } from "./common.js";
import { sleep } from "../util/wait.js";
import { waitFor } from "../util/wait.js";
import { log } from "../util/log.js";
import { nowMs } from "../util/time.js";

interface BacklogReport {
  /** Steady-state baseline ingest latency before SSP outage. */
  baseline: LatencySummary;
  baselineRatePerSec: number;
  /** Number of events ingested while SSP was down. */
  bufferedEvents: number;
  /** Number of those that the scheduler returned 200 for. */
  bufferedAccepted: number;
  /** Catch-up time series and headline numbers. */
  catchUp: CatchUpResult;
  /** Whether the SSP returned to ready_ssps>=1 within the window. */
  sspReturnedReady: boolean;
  /** Throughput observed during catch-up (events/sec). */
  catchUpThroughputPerSec: number;
  /** Replay/baseline ratio. >1 means catch-up faster than steady-state. */
  speedupRatio: number;
}

export interface BacklogArgs {
  rows: number;
  registeredQueries: number;
  baselineEvents: number;
  baselineRatePerSec: number;
  outageSecs: number;
  bufferedEvents: number;
  bufferedRatePerSec: number;
  catchUpTimeoutSec: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runBacklog(args: BacklogArgs): Promise<void> {
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const env = envSnapshot();
  const sha = await gitSha();

  const { stack, seedCounts } = await startSeededStack(args.rows, {
    verbose: args.verbose,
    surreal: { image: args.surrealImage, modulesDir: args.modulesDir, schemaPath: args.schemaPath },
    scheduler: { binPath: args.schedulerBin },
    ssp: { binPath: args.sspBin },
  });
  let activeStack = stack;
  try {
    const client = new SchedulerClient(activeStack.scheduler.baseUrl);

    log.info(`Pre-registering ${args.registeredQueries} background queries`);
    const queries = generateQueries(args.registeredQueries, `backlog-${randomUUID()}`);
    for (const q of queries) await client.registerView(q);
    const canaryId = `bench:backlog:${randomUUID()}`;
    await client.registerView({
      id: canaryId,
      surql: `SELECT * FROM comment`,
      clientId: `backlog-canary`,
      ttl: "1h",
      lastActiveAt: new Date().toISOString(),
    });

    // ---- Baseline ----
    log.step(`baseline: ${args.baselineEvents} events @ ${args.baselineRatePerSec}/s`);
    const baselineRec = new LatencyRecorder();
    const intervalMs = 1000 / args.baselineRatePerSec;
    const baselineEvents = generateIngests(
      args.baselineEvents,
      { users: Math.max(1, seedCounts.users), threads: Math.max(1, seedCounts.threads) },
      40_000_000,
    );
    const baselineStart = nowMs();
    for (const ev of baselineEvents) {
      const start = nowMs();
      await client.ingest(ev);
      baselineRec.record(nowMs() - start);
      const wait = intervalMs - (nowMs() - start);
      if (wait > 0) await sleep(wait);
    }
    const baselineWall = nowMs() - baselineStart;

    // ---- Outage ----
    log.step(`SSP outage: kill SSP, ingest ${args.bufferedEvents} events while down`);
    await activeStack.ssp.stop();
    // We do NOT wait for scheduler ready_ssps to drop, the scheduler's stale-SSP
    // detection runs on a 30s interval requiring 30s of missed heartbeats
    // (apps/scheduler/src/metrics.rs start_query_reassignment_monitor), so it
    // takes ~60s. Instead we proceed to ingest immediately. Events that fail
    // are counted; events accepted while the scheduler still thinks the SSP
    // is "ready" will fail to forward and may either be buffered or dropped
    // depending on transport semantics, that's part of what this suite
    // characterises.

    let bufferedAccepted = 0;
    const bufferedEvents = generateIngests(
      args.bufferedEvents,
      { users: Math.max(1, seedCounts.users), threads: Math.max(1, seedCounts.threads) },
      50_000_000,
    );
    const bufStart = nowMs();
    const bufInterval = 1000 / args.bufferedRatePerSec;
    for (const ev of bufferedEvents) {
      try {
        await client.ingest(ev);
        bufferedAccepted++;
      } catch {
        /* scheduler may push back; we record the count */
      }
      const wait = bufInterval - 0;
      if (wait > 0) await sleep(wait);
    }
    const bufWall = nowMs() - bufStart;
    log.info(
      `buffered ${bufferedAccepted}/${args.bufferedEvents} events in ${(bufWall / 1000).toFixed(1)}s while SSP was down`,
    );

    // Hold the outage for the configured duration if we ingested faster than required.
    const outageHold = args.outageSecs * 1000 - bufWall;
    if (outageHold > 0 && !args.smoke) {
      log.info(`holding outage for additional ${(outageHold / 1000).toFixed(1)}s`);
      await sleep(outageHold);
    }

    // ---- Restart + catch-up ----
    log.step(`restarting SSP and measuring catch-up`);
    activeStack = {
      ...activeStack,
      ssp: await activeStack.ssp.restart(),
    };
    // Wait for scheduler to mark SSP ready again.
    await waitFor(
      async () => {
        const m = await client.metrics().catch(() => null);
        return !!m && m.scheduler.ready_ssps >= 1;
      },
      { timeoutMs: 60_000, intervalMs: 250, label: "ready_ssps>=1 after restart" },
    );
    const catchUp = await awaitCatchUp(client, {
      intervalMs: 250,
      timeoutMs: args.catchUpTimeoutSec * 1000,
    });

    const baseline = baselineRec.summarize();
    const baselineRate =
      baselineWall > 0 ? (baselineRec.count() / baselineWall) * 1000 : 0;
    const catchUpThroughput = catchUp.replayRatePerSec;
    const speedup = baselineRate > 0 ? catchUpThroughput / baselineRate : 0;

    const report: BacklogReport = {
      baseline,
      baselineRatePerSec: baselineRate,
      bufferedEvents: args.bufferedEvents,
      bufferedAccepted,
      catchUp,
      sspReturnedReady: true,
      catchUpThroughputPerSec: catchUpThroughput,
      speedupRatio: speedup,
    };

    fs.mkdirSync(args.outDir, { recursive: true });
    const meta = {
      suite: "backlog",
      domain: "4.1 backlog recovery",
      startedAt,
      finishedAt: new Date().toISOString(),
      durationSec: (Date.now() - t0) / 1000,
      config: {
        rows: args.rows,
        registeredQueries: args.registeredQueries,
        baselineEvents: args.baselineEvents,
        baselineRatePerSec: args.baselineRatePerSec,
        outageSecs: args.outageSecs,
        bufferedEvents: args.bufferedEvents,
        bufferedRatePerSec: args.bufferedRatePerSec,
        catchUpTimeoutSec: args.catchUpTimeoutSec,
      },
      env: { ...env, gitSha: sha },
    };
    fs.writeFileSync(
      path.join(args.outDir, "samples.json"),
      JSON.stringify({ meta, report }, null, 2),
    );
    fs.writeFileSync(path.join(args.outDir, "run.json"), JSON.stringify(meta, null, 2));
    fs.writeFileSync(path.join(args.outDir, "summary.md"), renderMarkdown(meta, report));
    log.info(`Report written to ${args.outDir}`);
  } finally {
    await safeStop(activeStack);
  }
}

function fmt(n: number): string {
  if (!Number.isFinite(n) || n === 0) return "n/a";
  return n.toFixed(2);
}

function renderMarkdown(meta: any, r: BacklogReport): string {
  const lines: string[] = [];
  lines.push(`# backlog (Domain 4.1, outage and catch-up)`);
  lines.push("");
  lines.push(
    `Establish steady-state, kill the SSP, ingest ${meta.config.bufferedEvents} events while down, then restart the SSP and measure how fast the scheduler drains its backlog into the SSP.`,
  );
  lines.push("");
  lines.push(`**Observations on current sp00ky outage handling**:`);
  lines.push("");
  lines.push(
    `- Scheduler stale-SSP detection runs every 30s and demotes an SSP only after 30s of missed heartbeats (\`apps/scheduler/src/metrics.rs\`). For short outages, the scheduler keeps trying to forward to the dead SSP and logs failed forwards; events accepted in this window may be dropped at the forwarder rather than buffered.`,
  );
  lines.push(
    `- The per-SSP message buffer (\`apps/scheduler/src/router.rs:113-141\`) only engages when an SSP is in \`Bootstrapping\` or \`Replaying\` state. A "ready" SSP that's dead doesn't trigger buffering.`,
  );
  lines.push(
    `- After a restart, the scheduler's integrity check may force the new SSP to re-bootstrap by exiting (\`SSP exited code=4\` / \`Scheduler requested re-bootstrap\`). Production deployments rely on an external supervisor to respawn the SSP through this loop; the benchmark stops at the first exit and reports observed lag at timeout.`,
  );
  lines.push("");
  lines.push(
    `Conclusion: a clean kill→catch-up loop on a single host requires external SSP supervision. Numbers below characterise what's directly observable.`,
  );
  lines.push("");
  lines.push(`## Steady-state baseline`);
  lines.push("");
  lines.push(`Sustained rate: ${r.baselineRatePerSec.toFixed(0)} events/sec`);
  lines.push("");
  lines.push(`| metric | p50 | p95 | p99 | max |`);
  lines.push(`| --- | ---: | ---: | ---: | ---: |`);
  lines.push(
    `| ingest latency (ms) | ${fmt(r.baseline.p50)} | ${fmt(r.baseline.p95)} | ${fmt(r.baseline.p99)} | ${fmt(r.baseline.max)} |`,
  );
  lines.push("");
  lines.push(`## During outage`);
  lines.push("");
  lines.push(
    `${r.bufferedAccepted}/${r.bufferedEvents} events accepted by the scheduler while no SSP was attached. Failed events: ${r.bufferedEvents - r.bufferedAccepted}.`,
  );
  lines.push("");
  lines.push(`## Catch-up`);
  lines.push("");
  if (!r.catchUp.caughtUp) {
    lines.push(
      `**Did not reach lag=0 within ${meta.config.catchUpTimeoutSec}s.** Initial backlog: ${r.catchUp.initialBacklog}, last observed lag: ${r.catchUp.samples.at(-1)?.lag ?? "n/a"}.`,
    );
  } else {
    lines.push(`Reached lag=0 in ${(r.catchUp.catchUpMs / 1000).toFixed(2)}s.`);
    lines.push(`Replay throughput: ${r.catchUpThroughputPerSec.toFixed(0)} events/sec.`);
    lines.push(`Speedup ratio (replay / steady-state): ${r.speedupRatio.toFixed(2)}×.`);
  }
  lines.push("");
  return lines.join("\n");
}
