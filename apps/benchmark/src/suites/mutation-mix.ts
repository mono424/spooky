import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { startSeededStack, safeStop } from "./common.js";
import { makeMutationGenerator, type MutationMix } from "../workload/mutations.js";
import { generateQueries } from "../workload/queries.js";
import { log } from "../util/log.js";
import { nowMs } from "../util/time.js";

interface PhaseResult {
  name: string;
  mix: MutationMix;
  /** Latency by op for this phase. */
  byOp: Record<"CREATE" | "UPDATE" | "DELETE", LatencySummary>;
  /** Aggregate latency across all ops in this phase. */
  overall: LatencySummary;
  count: number;
  wallMs: number;
}

export interface MutationMixArgs {
  rows: number;
  registeredQueries: number;
  eventsPerPhase: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runMutationMix(args: MutationMixArgs): Promise<void> {
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

    // Background load: register some queries so the SSP isn't trivially idle.
    const queries = generateQueries(args.registeredQueries, `mut-bg-${randomUUID()}`);
    for (const q of queries) await client.registerView(q);

    // NOTE: this suite measures *accept latency* (`POST /ingest` → 200), NOT
    // end-to-end propagation. Why: a plain `SELECT * FROM comment` canary's
    // `last_hash` doesn't bump on UPDATE because UPDATE has weight 0 in DBSP
    // (membership unchanged) and `last_hash` for non-subquery views only
    // recomputes on cache mutation. End-to-end UPDATE timing therefore can't
    // be observed via the canary path; accept latency is the closest signal
    // that's consistently measurable for all three op types.

    // Pre-seed some comment ids so UPDATE/DELETE have real targets. Use the
    // existing seeder's id pattern (`comment:0`..`comment:N-1`).
    const existingIds = Array.from({ length: Math.min(seedCounts.comments, 1000) }, (_, i) => `comment:${i}`);

    const phases: { name: string; mix: MutationMix }[] = [
      { name: "100% CREATE", mix: { create: 1, update: 0, delete: 0 } },
      { name: "100% UPDATE", mix: { create: 0, update: 1, delete: 0 } },
      { name: "100% DELETE", mix: { create: 0, update: 0, delete: 1 } },
      { name: "70/20/10 mix", mix: { create: 0.7, update: 0.2, delete: 0.1 } },
    ];

    const results: PhaseResult[] = [];
    for (const phase of phases) {
      log.step(`mutation-mix phase: ${phase.name} × ${args.eventsPerPhase}`);
      const gen = makeMutationGenerator(args.eventsPerPhase, phase.mix, [...existingIds], {
        users: seedCounts.users,
        threads: seedCounts.threads,
      });

      const recCREATE = new LatencyRecorder();
      const recUPDATE = new LatencyRecorder();
      const recDELETE = new LatencyRecorder();
      const recAll = new LatencyRecorder();

      const phaseStart = nowMs();
      for (let i = 0; i < gen.size; i++) {
        const ev = gen.next();
        const start = nowMs();
        await client.ingest(ev);
        const ms = nowMs() - start;
        recAll.record(ms);
        if (ev._op === "CREATE") recCREATE.record(ms);
        else if (ev._op === "UPDATE") recUPDATE.record(ms);
        else recDELETE.record(ms);
      }
      const wallMs = nowMs() - phaseStart;

      results.push({
        name: phase.name,
        mix: phase.mix,
        byOp: {
          CREATE: recCREATE.summarize(),
          UPDATE: recUPDATE.summarize(),
          DELETE: recDELETE.summarize(),
        },
        overall: recAll.summarize(),
        count: args.eventsPerPhase,
        wallMs,
      });
    }

    fs.mkdirSync(args.outDir, { recursive: true });
    const meta = {
      suite: "mutation-mix",
      domain: "1.2 mutation overhead",
      startedAt,
      finishedAt: new Date().toISOString(),
      durationSec: (Date.now() - t0) / 1000,
      config: {
        rows: args.rows,
        registeredQueries: args.registeredQueries,
        eventsPerPhase: args.eventsPerPhase,
      },
      env: { ...env, gitSha: sha },
    };
    fs.writeFileSync(
      path.join(args.outDir, "samples.json"),
      JSON.stringify({ meta, phases: results }, null, 2),
    );
    fs.writeFileSync(path.join(args.outDir, "run.json"), JSON.stringify(meta, null, 2));
    fs.writeFileSync(path.join(args.outDir, "summary.md"), renderMarkdown(meta, results));
    log.info(`Report written to ${args.outDir}`);
  } finally {
    await safeStop(stack);
  }
}

function fmt(n: number): string {
  if (!Number.isFinite(n) || n === 0) return "n/a";
  if (n >= 1000) return n.toFixed(0);
  return n.toFixed(2);
}

function renderMarkdown(meta: any, phases: PhaseResult[]): string {
  const lines: string[] = [];
  lines.push(`# mutation-mix (Domain 1.2, INSERT vs UPDATE vs DELETE)`);
  lines.push("");
  lines.push(
    `Each phase runs ${meta.config.eventsPerPhase} mutations of a fixed mix on the \`comment\` table. **Latencies are accept-latency** (\`POST /ingest\` → scheduler returns 200), not end-to-end propagation. UPDATE events have weight 0 in DBSP (membership unchanged), so a plain \`SELECT * FROM comment\` canary's \`last_hash\` doesn't bump on UPDATE, there's no observable end-to-end signal for UPDATE on a non-subquery view. Accept latency is the closest signal consistently measurable across CREATE/UPDATE/DELETE.`,
  );
  lines.push("");
  for (const p of phases) {
    lines.push(`## ${p.name}`);
    lines.push("");
    lines.push(`Mix: \`${JSON.stringify(p.mix)}\`, ${p.count} events in ${(p.wallMs / 1000).toFixed(1)}s`);
    lines.push("");
    lines.push(`| op | count | p50 | p95 | p99 | max |`);
    lines.push(`| --- | ---: | ---: | ---: | ---: | ---: |`);
    const order: Array<keyof typeof p.byOp> = ["CREATE", "UPDATE", "DELETE"];
    for (const op of order) {
      const l = p.byOp[op];
      if (l.count === 0) continue;
      lines.push(`| ${op} | ${l.count} | ${fmt(l.p50)} | ${fmt(l.p95)} | ${fmt(l.p99)} | ${fmt(l.max)} |`);
    }
    lines.push(`| **all** | ${p.overall.count} | ${fmt(p.overall.p50)} | ${fmt(p.overall.p95)} | ${fmt(p.overall.p99)} | ${fmt(p.overall.max)} |`);
    lines.push("");
  }
  return lines.join("\n");
}
