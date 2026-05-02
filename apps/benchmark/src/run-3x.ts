#!/usr/bin/env node
/**
 * Runs each entry in MATRIX three times and writes an averaged report at
 * `results/aggregated.md`. Each individual run still produces its own
 * `samples.json` / `summary.md` under `results/<entry.label>-rN/`, so the
 * raw data is preserved for inspection.
 */
import fs from "node:fs";
import path from "node:path";
import { execa } from "execa";
import { log } from "./util/log.js";

interface Row {
  metric: string;
  values: number[];
}

interface RunSummary {
  rows: Row[];
}

interface MatrixEntry {
  label: string;
  suite: string;
  args: string[];
  /** Returns rows of (metric, value-per-step) for one suite's samples.json. */
  parse: (samples: any) => Row[];
}

const RESULTS_ROOT = path.resolve(process.cwd(), "results");
const RUN_COUNT = 3;

const MATRIX: MatrixEntry[] = [
  // Realistic-row sweep: ~1 KiB per comment (typical short post),
  // row count varies to hit each total DB size. Capped at 30k rows
  // because the canary `SELECT * FROM comment` materialisation step
  // fails (HTTP 500 on /view/register) once it has to walk ~36k+
  // matched rows on this hardware, regardless of byte volume.
  ...[
    { rows: 1_000, label: "real-1mb" },
    { rows: 5_000, label: "real-5mb" },
    { rows: 10_000, label: "real-10mb" },
    { rows: 20_000, label: "real-20mb" },
    { rows: 30_000, label: "real-30mb" },
  ].map(({ rows, label }) => ({
    label,
    suite: "subquery-depth",
    args: [
      "--rows", String(rows),
      "--row-bytes", "900",
      "--ingest-events", "30",
    ],
    parse: parseSubqueryDepth,
  })),

  // Fixed-row-count sweep: 1k rows, growing per-row size up to ~1 GiB.
  // Tests the orthogonal axis: per-event work vs raw byte volume in the
  // store. Higher row counts are cut off by the view-register ceiling
  // above; this axis isolates "data is large but few records match".
  ...[
    { rowBytes: 0, label: "fat-100kb" },
    { rowBytes: 5_000, label: "fat-3mb" },
    { rowBytes: 50_000, label: "fat-30mb" },
    { rowBytes: 100_000, label: "fat-60mb" },
    { rowBytes: 200_000, label: "fat-120mb" },
    { rowBytes: 500_000, label: "fat-250mb" },
    { rowBytes: 1_000_000, label: "fat-500mb" },
    { rowBytes: 2_000_000, label: "fat-1gb" },
  ].map(({ rowBytes, label }) => ({
    label,
    suite: "subquery-depth",
    args: ["--rows", "1000", "--row-bytes", String(rowBytes), "--ingest-events", "30"],
    parse: parseSubqueryDepth,
  })),

  // Standard suites at default-ish configs.
  {
    label: "velocity",
    suite: "velocity",
    args: ["--rates", "100,200,500,1000,2000", "--rows", "1000", "--hold-secs", "3"],
    parse: parseVelocity,
  },
  {
    label: "mutation-mix",
    suite: "mutation-mix",
    args: ["--rows", "1000", "--events-per-phase", "300"],
    parse: parseMutationMix,
  },
  {
    label: "join-fanout",
    suite: "join-fanout",
    args: ["--fanouts", "1,10,100,500,1000,2000", "--iterations", "20"],
    parse: parseJoinFanout,
  },
  {
    label: "view-density",
    suite: "view-density",
    args: ["--views", "100,500,1000,2000", "--rows", "500", "--ingest-events", "100"],
    parse: parseViewDensity,
  },
];

function parseSubqueryDepth(samples: any): Row[] {
  const rows: Row[] = [];
  for (const r of samples.results ?? []) {
    if (!r.registered) continue;
    rows.push({ metric: `depth-${r.depth} p50`, values: [r.latency.p50] });
    rows.push({ metric: `depth-${r.depth} p95`, values: [r.latency.p95] });
  }
  return rows;
}

function parseVelocity(samples: any): Row[] {
  const rows: Row[] = [];
  for (const s of samples.steps ?? []) {
    rows.push({
      metric: `target=${s.targetRatePerSec} achieved`,
      values: [s.achievedRatePerSec],
    });
    rows.push({ metric: `target=${s.targetRatePerSec} p50`, values: [s.acceptLatency.p50] });
    rows.push({ metric: `target=${s.targetRatePerSec} p95`, values: [s.acceptLatency.p95] });
  }
  return rows;
}

function parseMutationMix(samples: any): Row[] {
  const rows: Row[] = [];
  for (const phase of samples.phases ?? []) {
    for (const op of ["CREATE", "UPDATE", "DELETE"] as const) {
      const lat = phase.byOp?.[op];
      if (!lat || lat.count === 0) continue;
      rows.push({ metric: `${phase.name} ${op} p50`, values: [lat.p50] });
      rows.push({ metric: `${phase.name} ${op} p95`, values: [lat.p95] });
    }
  }
  return rows;
}

function parseJoinFanout(samples: any): Row[] {
  const rows: Row[] = [];
  for (const s of samples.steps ?? []) {
    if (!s.registered) continue;
    rows.push({ metric: `K=${s.fanout} settle`, values: [s.meanSettleMs] });
  }
  return rows;
}

function parseViewDensity(samples: any): Row[] {
  const rows: Row[] = [];
  for (const s of samples.steps ?? []) {
    rows.push({ metric: `views=${s.views} reg-p50`, values: [s.registration.p50] });
    rows.push({ metric: `views=${s.views} ing-p50`, values: [s.ingest.p50] });
    rows.push({ metric: `views=${s.views} ing-p95`, values: [s.ingest.p95] });
    rows.push({
      metric: `views=${s.views} RSS-MiB`,
      values: [s.rssEndKib / 1024],
    });
  }
  return rows;
}

async function main() {
  const cliPath = path.resolve(import.meta.dirname, "index.js");
  fs.mkdirSync(RESULTS_ROOT, { recursive: true });
  const aggregated: { label: string; rows: Row[] }[] = [];

  for (const entry of MATRIX) {
    log.step(`${entry.label}: ${RUN_COUNT}× ${entry.suite} ${entry.args.join(" ")}`);
    const merged = new Map<string, number[]>();
    for (let r = 1; r <= RUN_COUNT; r++) {
      const out = path.join(RESULTS_ROOT, `${entry.label}-r${r}`);
      fs.rmSync(out, { recursive: true, force: true });
      log.info(`  run ${r}/${RUN_COUNT} ...`);
      try {
        await execa("node", [cliPath, entry.suite, ...entry.args, "--out", out], {
          stdio: "inherit",
        });
      } catch (e) {
        log.warn(`  run ${r} failed: ${(e as Error).message}`);
        continue;
      }
      const samplesPath = path.join(out, "samples.json");
      if (!fs.existsSync(samplesPath)) continue;
      const samples = JSON.parse(fs.readFileSync(samplesPath, "utf8"));
      const rows = entry.parse(samples);
      for (const row of rows) {
        const arr = merged.get(row.metric) ?? [];
        arr.push(...row.values);
        merged.set(row.metric, arr);
      }
    }
    aggregated.push({
      label: entry.label,
      rows: Array.from(merged.entries()).map(([metric, values]) => ({ metric, values })),
    });
  }

  // Render aggregated.md
  const lines: string[] = [];
  lines.push(`# Averaged across ${RUN_COUNT} runs per config`);
  lines.push("");
  lines.push(`Generated: ${new Date().toISOString()}`);
  lines.push("");
  for (const block of aggregated) {
    lines.push(`## ${block.label}`);
    lines.push("");
    lines.push(`| metric | runs | mean | min | max | spread |`);
    lines.push(`| --- | ---: | ---: | ---: | ---: | ---: |`);
    for (const row of block.rows) {
      const vals = row.values;
      if (vals.length === 0) {
        lines.push(`| ${row.metric} | 0 | n/a | n/a | n/a | n/a |`);
        continue;
      }
      const mean = vals.reduce((a, b) => a + b, 0) / vals.length;
      const min = Math.min(...vals);
      const max = Math.max(...vals);
      const spread = mean > 0 ? `${(((max - min) / mean) * 100).toFixed(0)}%` : "n/a";
      lines.push(
        `| ${row.metric} | ${vals.length} | ${fmt(mean)} | ${fmt(min)} | ${fmt(max)} | ${spread} |`,
      );
    }
    lines.push("");
  }
  const outPath = path.join(RESULTS_ROOT, "aggregated.md");
  fs.writeFileSync(outPath, lines.join("\n"));
  log.info(`\nWrote ${outPath}`);
}

function fmt(n: number): string {
  if (!Number.isFinite(n)) return "n/a";
  if (Math.abs(n) >= 1000) return n.toFixed(0);
  if (Math.abs(n) >= 100) return n.toFixed(1);
  return n.toFixed(2);
}

main().catch((e) => {
  log.error(e instanceof Error ? e.stack ?? e.message : String(e));
  process.exit(1);
});
