import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execa } from "execa";
import type { LatencySummary } from "./latency.js";
import type { RssSample } from "./rss.js";
import type { SchedulerSnapshot } from "./scheduler-stats.js";

export interface StepResult {
  step: number;
  inputs: {
    queries: number;
    rows: number;
    ingestsPerStep: number;
    warmup: number;
  };
  registration: LatencySummary;
  /**
   * End-to-end ingest latency: from `POST /ingest` to the SSP's canary view
   * reflecting the change (last_hash advanced). Includes scheduler buffer +
   * forward + SSP DBSP apply + view materialization.
   */
  ingest: LatencySummary;
  /** Number of ingests whose view-update wait hit the timeout. */
  ingestTimeouts: number;
  registrationThroughputPerSec: number;
  ingestThroughputPerSec: number;
  rss: { samples: RssSample[]; minKib: number; maxKib: number; lastKib: number };
  schedulerSnapshots: SchedulerSnapshot[];
  schedulerLast: SchedulerSnapshot | null;
}

export interface RunMeta {
  suite: string;
  startedAt: string;
  finishedAt: string;
  durationSec: number;
  axis: "queries" | "rows";
  config: Record<string, unknown>;
  env: {
    node: string;
    platform: string;
    arch: string;
    cpu: string;
    cpuCount: number;
    totalMemMib: number;
    gitSha?: string;
  };
}

export async function gitSha(): Promise<string | undefined> {
  try {
    const { stdout, exitCode } = await execa("git", ["rev-parse", "--short", "HEAD"], {
      reject: false,
    });
    if (exitCode === 0) return stdout.trim();
  } catch {
    /* ignore */
  }
  return undefined;
}

export function envSnapshot(): RunMeta["env"] {
  const cpu = os.cpus()[0]?.model ?? "unknown";
  return {
    node: process.version,
    platform: process.platform,
    arch: process.arch,
    cpu,
    cpuCount: os.cpus().length,
    totalMemMib: Math.round(os.totalmem() / 1024 / 1024),
  };
}

export interface WriteReportArgs {
  outDir: string;
  meta: RunMeta;
  steps: StepResult[];
}

export async function writeReport(args: WriteReportArgs): Promise<void> {
  fs.mkdirSync(args.outDir, { recursive: true });
  const samplesPath = path.join(args.outDir, "samples.json");
  const runPath = path.join(args.outDir, "run.json");
  const summaryPath = path.join(args.outDir, "summary.md");

  fs.writeFileSync(
    samplesPath,
    JSON.stringify(
      {
        meta: args.meta,
        steps: args.steps,
      },
      null,
      2,
    ),
  );

  fs.writeFileSync(runPath, JSON.stringify(args.meta, null, 2));

  fs.writeFileSync(summaryPath, renderMarkdown(args.meta, args.steps));
}

function fmt(n: number): string {
  if (!Number.isFinite(n)) return "n/a";
  if (n >= 1000) return n.toFixed(0);
  if (n >= 100) return n.toFixed(1);
  return n.toFixed(2);
}

function fmtKiB(kib: number): string {
  if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(2)} GiB`;
  if (kib >= 1024) return `${(kib / 1024).toFixed(1)} MiB`;
  return `${kib} KiB`;
}

function renderMarkdown(meta: RunMeta, steps: StepResult[]): string {
  const axis = meta.axis;
  const lines: string[] = [];
  lines.push(`# sp00ky benchmark, ${meta.suite}`);
  lines.push("");
  lines.push(`- Started: ${meta.startedAt}`);
  lines.push(`- Finished: ${meta.finishedAt} (${meta.durationSec.toFixed(1)}s)`);
  lines.push(
    `- Env: ${meta.env.platform}/${meta.env.arch}, ${meta.env.cpuCount}× ${meta.env.cpu}, ${meta.env.totalMemMib} MiB RAM, Node ${meta.env.node}` +
      (meta.env.gitSha ? `, git ${meta.env.gitSha}` : ""),
  );
  lines.push(`- Config: \`${JSON.stringify(meta.config)}\``);
  lines.push("");

  const fixedLabel = axis === "queries" ? "rows (fixed)" : "queries (fixed)";

  lines.push(`## Registration latency (per /view/register call)`);
  lines.push("");
  lines.push(
    `| ${axis} | ${fixedLabel} | count | min | p50 | p95 | p99 | max | throughput (ops/s) |`,
  );
  lines.push(`| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |`);
  for (const s of steps) {
    const primary = axis === "queries" ? s.inputs.queries : s.inputs.rows;
    const other = axis === "queries" ? s.inputs.rows : s.inputs.queries;
    const r = s.registration;
    lines.push(
      `| ${primary} | ${other} | ${r.count} | ${fmt(r.min)} | ${fmt(r.p50)} | ${fmt(r.p95)} | ${fmt(r.p99)} | ${fmt(r.max)} | ${fmt(s.registrationThroughputPerSec)} |`,
    );
  }
  lines.push("");

  lines.push(`## Ingest latency (POST /ingest → canary view updated)`);
  lines.push("");
  lines.push(
    `| ${axis} | ${fixedLabel} | count | timeouts | min | p50 | p95 | p99 | max | throughput (ops/s) |`,
  );
  lines.push(`| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |`);
  for (const s of steps) {
    const primary = axis === "queries" ? s.inputs.queries : s.inputs.rows;
    const other = axis === "queries" ? s.inputs.rows : s.inputs.queries;
    const r = s.ingest;
    lines.push(
      `| ${primary} | ${other} | ${r.count} | ${s.ingestTimeouts} | ${fmt(r.min)} | ${fmt(r.p50)} | ${fmt(r.p95)} | ${fmt(r.p99)} | ${fmt(r.max)} | ${fmt(s.ingestThroughputPerSec)} |`,
    );
  }
  lines.push("");

  lines.push(`## SSP RSS`);
  lines.push("");
  lines.push(`| ${axis} | min | max | last |`);
  lines.push(`| ---: | ---: | ---: | ---: |`);
  for (const s of steps) {
    const primary = axis === "queries" ? s.inputs.queries : s.inputs.rows;
    lines.push(
      `| ${primary} | ${fmtKiB(s.rss.minKib)} | ${fmtKiB(s.rss.maxKib)} | ${fmtKiB(s.rss.lastKib)} |`,
    );
  }
  lines.push("");

  lines.push(`## Scheduler /metrics (last snapshot per step)`);
  lines.push("");
  lines.push(
    `| ${axis} | total_queries | running_jobs | pending_events | snapshot_seq | latest_seq | lag |`,
  );
  lines.push(`| ---: | ---: | ---: | ---: | ---: | ---: | ---: |`);
  for (const s of steps) {
    const primary = axis === "queries" ? s.inputs.queries : s.inputs.rows;
    const m = s.schedulerLast?.metrics.scheduler;
    if (!m) {
      lines.push(`| ${primary} |, |, |, |, |, |, |`);
    } else {
      lines.push(
        `| ${primary} | ${m.total_queries} | ${m.running_jobs} | ${m.pending_events} | ${m.snapshot_seq} | ${m.latest_seq} | ${m.lag} |`,
      );
    }
  }
  lines.push("");

  lines.push(renderMethodology(meta));

  return lines.join("\n");
}

/**
 * Verbatim explanation of what each table above measures, how it's measured,
 * and what's specifically excluded. Lives at the bottom of every report so a
 * reader can interpret the numbers without context outside the file.
 */
function renderMethodology(meta: RunMeta): string {
  const axis = meta.axis;
  const swept = axis === "queries" ? "registered query count" : "DB row count";
  const fixed = axis === "queries" ? "DB row count" : "registered query count";

  return [
    `## What these numbers mean`,
    "",
    `### Stack under test`,
    "",
    `Each step runs against a **freshly spawned, hermetic stack** torn down before the next step:`,
    "",
    `1. **SurrealDB v3** in a Docker container (testcontainers, schemaless namespace + database created at startup, the user-authored schema at \`tests/schema.surql\` applied: \`user\`, \`thread\`, \`comment\`, plus one RELATE event).`,
    `2. **Scheduler** (release-built \`apps/scheduler\` binary) listening on \`127.0.0.1:9667\`, configured to talk to the SurrealDB container above.`,
    `3. **One SSP** (release-built \`apps/ssp\` binary) listening on \`127.0.0.1:8667\`, registered with the scheduler. Multi-SSP topologies are out of scope for this suite.`,
    "",
    `Both the scheduler and SSP share \`SPKY_AUTH_SECRET\` so the scheduler can post to SSP authenticated routes (\`/view/register\`, \`/ingest\`, \`/debug/view/:id\`).`,
    "",
    `### Workload per step`,
    "",
    `1. Seed the SurrealDB instance with \`rows\` rows split across the three tables (10% \`user\`, 30% \`thread\`, 60% \`comment\`) using batched \`INSERT INTO\` statements.`,
    `2. Start the SSP RSS sampler and the scheduler \`/metrics\` poller (background tasks).`,
    `3. **Warmup**: register \`warmup\` throwaway queries via the scheduler, then unregister them. Their latencies are discarded.`,
    `4. **Measure registration**: register \`queries\` distinct \`SELECT\` queries (rotating across \`user\`/\`thread\`/\`comment\`) one-at-a-time via \`POST /view/register\` and record per-call latency.`,
    `5. **Register one canary view** (\`SELECT * FROM comment\`) used as the propagation oracle for ingests. Capture its initial \`last_hash\` from the SSP's \`GET /debug/view/:id\`.`,
    `6. **Measure ingest**: send \`ingestsPerStep\` \`CREATE\` events targeting the \`comment\` table via \`POST /ingest\` one-at-a-time. After each, poll the canary view's \`last_hash\` until it differs from the previous value. Record per-call latency from request-send to first changed-hash observation.`,
    `7. Take one synchronous \`/metrics\` snapshot, stop the samplers, tear down the stack.`,
    "",
    `### Sweep`,
    "",
    `This run sweeps **${swept}** while holding **${fixed}** fixed. Each row in the tables above corresponds to one (queries, rows) point.`,
    "",
    `### Metric definitions`,
    "",
    `**Registration latency**: wall-clock time from sending \`POST /view/register\` (with \`Authorization: Bearer …\`) to the scheduler's HTTP response. The scheduler internally selects an SSP, forwards the request synchronously to that SSP, waits for the SSP to finish its initial bootstrap query against SurrealDB, and only then returns to the caller. So this latency includes: scheduler accept + SSP selection + scheduler→SSP HTTP forward + SSP bootstrap query against SurrealDB + return path. Concurrency = 1.`,
    "",
    `**Ingest latency**: wall-clock time from sending \`POST /ingest\` to the **first observation** that the canary view's \`last_hash\` differs from the value seen before the request. \`last_hash\` is computed inside the SSP's DBSP circuit and only changes when the materialized view content has actually been updated. So this includes: scheduler accept + WAL append + scheduler buffer drain + scheduler→SSP HTTP forward + SSP DBSP circuit apply + view materialization. Concurrency = 1.`,
    "",
    `**Polling resolution**: the canary-view poll loop yields with \`setImmediate\` between requests, so polls run at HTTP RTT speed (~0.3 ms locally on the loopback interface). The reported ingest latency is therefore quantized to roughly one poll RTT; sub-millisecond propagation cannot be distinguished from "one poll round-trip".`,
    "",
    `**timeouts**: number of ingest events whose \`last_hash\` did not change within a 10 s wait window. A non-zero value means at least that many ingests either did not propagate or coalesced with another event. The latency we record for a timed-out event is the full 10 s, which will skew \`max\`/\`p99\` upward.`,
    "",
    `**throughput (ops/s)**: \`count / wall_clock_ms × 1000\`, where \`wall_clock_ms\` is the elapsed time across the entire serial loop (start of the first call to end of the last). This is sustained one-at-a-time throughput, not max throughput; concurrent drivers would produce higher numbers.`,
    "",
    `**SSP RSS (min/max/last)**: Resident Set Size of the SSP child process in KiB, sampled by \`ps -o rss= -p <pid>\` at 250 ms cadence between the start of the warmup and the end of the ingest phase. \`min\` and \`max\` cover the entire window; \`last\` is the final sample before the SSP was killed. RSS reflects all SSP allocations: DBSP circuit graph + per-view caches + heartbeat + connection pool + Tokio runtime. Memory growth between rows of the table is the per-query (or per-row) cost.`,
    "",
    `**Scheduler /metrics last snapshot**: the final \`GET /metrics\` payload captured for the step, taken synchronously **after** the ingest loop finishes and **after** the canary view has been unregistered. Fields:`,
    `- \`total_queries\`, registered queries currently tracked by the scheduler. Equals \`queries\` for the step (the canary view is registered transiently for ingest measurement and unregistered before this snapshot is taken, so it does not appear here).`,
    `- \`running_jobs\`, scheduler's job tracker count (background work like backups; expected to be 0 here).`,
    `- \`pending_events\`, events accepted by \`/ingest\` but not yet drained from the in-memory buffer.`,
    `- \`snapshot_seq\`, last sequence number applied to the scheduler's local replica.`,
    `- \`latest_seq\`, highest sequence number assigned at \`/ingest\`.`,
    `- \`lag\`, \`latest_seq − snapshot_seq\`. Non-zero \`lag\` does **not** mean the SSP hasn't seen the events; the SSP receives events directly from the scheduler's forwarder. \`lag\` reflects the scheduler-replica catch-up, which is independent of view propagation.`,
    "",
    `### Scope and known limitations`,
    "",
    `- **Concurrency = 1.** All measurements are one-call-at-a-time. This isolates per-call cost and avoids contention artifacts but cannot show "max throughput under load".`,
    `- **Single SSP.** Scheduler load-balancing across multiple SSPs is not exercised here.`,
    `- **Local-loopback network.** Scheduler ↔ SSP is \`127.0.0.1\`, so RTT is sub-millisecond. Real cluster deployments will have higher absolute numbers; the *deltas* between rows in this report are still informative.`,
    `- **Fresh stack per step.** RSS values are not cumulative across steps. To observe cumulative growth (e.g. RSS as N queries pile up on the same SSP), a future run mode that reuses the stack across steps is needed.`,
    `- **Canary view is transient.** Registered after the measured registration phase and unregistered before the post-step \`/metrics\` snapshot, so it neither appears in registration latencies nor inflates \`total_queries\`. While the ingest loop runs it is +1 to the SSP's view count, so memory/RSS measurements during ingest reflect \`queries + 1\` views.`,
    `- **Ingest workload is CREATE-only on \`comment\`.** Hash-based propagation detection works for any mutation, but UPDATE/DELETE patterns may exercise different DBSP paths and aren't covered here.`,
    "",
  ].join("\n");
}
