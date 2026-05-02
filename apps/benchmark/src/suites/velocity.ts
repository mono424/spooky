import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { runConcurrentIngest } from "../drivers/concurrent-ingest.js";
import { generateIngests } from "../workload/ingest.js";
import { generateQueries } from "../workload/queries.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { summarizeRamp, type RampStep } from "../metrics/threshold.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { startSeededStack, safeStop } from "./common.js";
import { log } from "../util/log.js";

export interface VelocityArgs {
  rates: number[];
  rows: number;
  registeredQueries: number;
  holdSecs: number;
  concurrency: number;
  thresholdMs: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runVelocity(args: VelocityArgs): Promise<void> {
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
  try {
    const client = new SchedulerClient(stack.scheduler.baseUrl);

    // Register a fixed set of queries so the SSP has realistic background load.
    const queries = generateQueries(args.registeredQueries, `velocity-${randomUUID()}`);
    log.info(`Pre-registering ${queries.length} background queries`);
    for (const q of queries) {
      await client.registerView(q);
    }

    const steps: RampStep[] = [];
    let ingestSeq = 5_000_000;
    for (const rate of args.rates) {
      log.step(`velocity step: target=${rate} ev/s for ${args.holdSecs}s`);
      const ev = generateIngests(1, seedCountsToScale(seedCounts), ingestSeq);
      ingestSeq += args.holdSecs * rate * 2; // reserve unique ids
      const factory = (() => {
        let i = 0;
        return () => {
          const e = generateIngests(1, seedCountsToScale(seedCounts), ingestSeq + i++);
          return e[0]!;
        };
      })();
      // ev unused; factory generates per-call.
      void ev;

      const result = await runConcurrentIngest(client, factory, {
        concurrency: args.concurrency,
        ratePerSec: rate,
        durationSec: args.holdSecs,
      });

      const rec = new LatencyRecorder();
      for (const ms of result.acceptLatenciesMs) rec.record(ms);
      const lat = rec.summarize();
      const withinThreshold = lat.p95 <= args.thresholdMs;

      steps.push({
        targetRatePerSec: rate,
        achievedRatePerSec: result.achievedRatePerSec,
        acceptLatency: lat,
        failed: result.failed,
        withinThreshold,
      });
      log.info(
        `target=${rate} achieved=${result.achievedRatePerSec.toFixed(0)} p50=${lat.p50.toFixed(2)}ms p95=${lat.p95.toFixed(2)}ms failed=${result.failed} → ${withinThreshold ? "OK" : "OVER"}`,
      );

      if (!withinThreshold && !args.smoke) {
        log.info(`Stopping ramp at first threshold breach`);
        break;
      }
    }

    const ramp = summarizeRamp(steps, args.thresholdMs);

    fs.mkdirSync(args.outDir, { recursive: true });
    const meta = {
      suite: "velocity",
      domain: "1.1 input velocity",
      startedAt,
      finishedAt: new Date().toISOString(),
      durationSec: (Date.now() - t0) / 1000,
      config: {
        rates: args.rates,
        rows: args.rows,
        registeredQueries: args.registeredQueries,
        holdSecs: args.holdSecs,
        concurrency: args.concurrency,
        thresholdMs: args.thresholdMs,
      },
      env: { ...env, gitSha: sha },
    };

    fs.writeFileSync(
      path.join(args.outDir, "samples.json"),
      JSON.stringify({ meta, ramp, steps }, null, 2),
    );
    fs.writeFileSync(path.join(args.outDir, "run.json"), JSON.stringify(meta, null, 2));
    fs.writeFileSync(path.join(args.outDir, "summary.md"), renderMarkdown(meta, steps, ramp));
    log.info(`Report written to ${args.outDir}`);
  } finally {
    await safeStop(stack);
  }
}

function seedCountsToScale(c: { users: number; threads: number }): {
  users: number;
  threads: number;
} {
  return { users: Math.max(1, c.users), threads: Math.max(1, c.threads) };
}

function fmt(n: number): string {
  if (!Number.isFinite(n)) return "n/a";
  if (n >= 1000) return n.toFixed(0);
  return n.toFixed(2);
}

function renderMarkdown(
  meta: ReturnType<typeof Object.assign>,
  steps: RampStep[],
  ramp: ReturnType<typeof summarizeRamp>,
): string {
  const lines: string[] = [];
  lines.push(`# velocity (Domain 1.1, input velocity)`);
  lines.push("");
  lines.push(`Step the concurrent /ingest rate up; record accept-latency p50/p95 per step.`);
  lines.push(`The "rate at threshold" is the first step where rolling p95 strictly exceeded \`${ramp.thresholdMs} ms\`.`);
  lines.push("");
  lines.push(`- Started: ${(meta as any).startedAt}`);
  lines.push(`- Config: \`${JSON.stringify((meta as any).config)}\``);
  lines.push(`- **rate at threshold**: ${ramp.rateAtThreshold ?? "never reached"}`);
  lines.push(`- **last sustained rate**: ${ramp.lastSustainedRate ?? "none"}`);
  lines.push("");
  lines.push(`| target ev/s | achieved ev/s | failed | accept p50 (ms) | p95 | p99 | max | within threshold |`);
  lines.push(`| ---: | ---: | ---: | ---: | ---: | ---: | ---: | :---: |`);
  for (const s of steps) {
    const l = s.acceptLatency;
    lines.push(
      `| ${s.targetRatePerSec} | ${fmt(s.achievedRatePerSec)} | ${s.failed} | ${fmt(l.p50)} | ${fmt(l.p95)} | ${fmt(l.p99)} | ${fmt(l.max)} | ${s.withinThreshold ? "✓" : "✗"} |`,
    );
  }
  lines.push("");
  lines.push(`## What this measures`);
  lines.push("");
  lines.push(
    `Each step runs a worker pool of ${(meta as any).config.concurrency} concurrent in-flight \`POST /ingest\` calls, paced to hit the target events/sec for ${(meta as any).config.holdSecs}s. Accept latency is from request-send to scheduler 200 response, it is NOT end-to-end propagation. End-to-end via canary polling is unreliable under concurrent ingest because overlapping events can't be attributed to a single hash transition; this suite measures the scheduler accept ceiling instead. The threshold check uses p95.`,
  );
  return lines.join("\n");
}
