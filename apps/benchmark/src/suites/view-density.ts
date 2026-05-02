import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { SspClient } from "../drivers/ssp-http.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { RssSampler, type RssSample } from "../metrics/rss.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { generateQueries } from "../workload/queries.js";
import { generateIngests } from "../workload/ingest.js";
import { startSeededStack, safeStop } from "./common.js";
import { log } from "../util/log.js";
import { nowMs } from "../util/time.js";

interface DensityStep {
  views: number;
  registration: LatencySummary;
  ingest: LatencySummary;
  ingestTimeouts: number;
  rssEndKib: number;
  rssMaxKib: number;
  rssSamples: RssSample[];
}

export interface ViewDensityArgs {
  views: number[];
  rows: number;
  rowBytes?: number;
  ingestEvents: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runViewDensity(args: ViewDensityArgs): Promise<void> {
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const env = envSnapshot();
  const sha = await gitSha();

  const steps: DensityStep[] = [];
  for (const viewCount of args.views) {
    log.step(`view-density: views=${viewCount}`);
    const { stack, seedCounts } = await startSeededStack(
      args.rows,
      {
        verbose: args.verbose,
        surreal: { image: args.surrealImage, modulesDir: args.modulesDir, schemaPath: args.schemaPath },
        scheduler: { binPath: args.schedulerBin },
        ssp: { binPath: args.sspBin },
      },
      { commentPadBytes: args.rowBytes },
    );
    try {
      const client = new SchedulerClient(stack.scheduler.baseUrl);
      const ssp = new SspClient(stack.ssp.baseUrl, stack.authSecret);

      const rss = new RssSampler(stack.ssp.pid ?? -1, 250);
      if (stack.ssp.pid) rss.start();

      // Measured registration phase.
      const regRec = new LatencyRecorder();
      const queries = generateQueries(viewCount, `density-${viewCount}-${randomUUID()}`);
      for (const q of queries) {
        const start = nowMs();
        await client.registerView(q);
        regRec.record(nowMs() - start);
      }

      // Canary + ingest phase.
      const canaryId = `bench:density:${viewCount}:${randomUUID()}`;
      await client.registerView({
        id: canaryId,
        surql: `SELECT * FROM comment`,
        clientId: `density-canary`,
        ttl: "1h",
        lastActiveAt: new Date().toISOString(),
      });
      let lastHash = (await ssp.getDebugView(canaryId))?.last_hash ?? "";
      const ingRec = new LatencyRecorder();
      let timeouts = 0;
      const ingests = generateIngests(
        args.ingestEvents,
        { users: Math.max(1, seedCounts.users), threads: Math.max(1, seedCounts.threads) },
        30_000_000 + viewCount * 100_000,
      );
      for (const ev of ingests) {
        const start = nowMs();
        await client.ingest(ev);
        const deadline = nowMs() + 10_000;
        let advanced = false;
        while (nowMs() < deadline) {
          const v = await ssp.getDebugView(canaryId).catch(() => null);
          if (v && v.last_hash && v.last_hash !== lastHash) {
            lastHash = v.last_hash;
            advanced = true;
            break;
          }
          await new Promise((r) => setImmediate(r));
        }
        ingRec.record(nowMs() - start);
        if (!advanced) timeouts++;
      }

      await rss.stop();
      const summary = rss.summary();
      steps.push({
        views: viewCount,
        registration: regRec.summarize(),
        ingest: ingRec.summarize(),
        ingestTimeouts: timeouts,
        rssEndKib: summary.lastKib,
        rssMaxKib: summary.maxKib,
        rssSamples: summary.samples,
      });
    } finally {
      await safeStop(stack);
    }
  }

  fs.mkdirSync(args.outDir, { recursive: true });
  const meta = {
    suite: "view-density",
    domain: "3.2 view density (multi-tenant scaling)",
    startedAt,
    finishedAt: new Date().toISOString(),
    durationSec: (Date.now() - t0) / 1000,
    config: { views: args.views, rows: args.rows, ingestEvents: args.ingestEvents },
    env: { ...env, gitSha: sha },
  };
  fs.writeFileSync(
    path.join(args.outDir, "samples.json"),
    JSON.stringify({ meta, steps }, null, 2),
  );
  fs.writeFileSync(path.join(args.outDir, "run.json"), JSON.stringify(meta, null, 2));
  fs.writeFileSync(path.join(args.outDir, "summary.md"), renderMarkdown(meta, steps));
  log.info(`Report written to ${args.outDir}`);
}

function fmt(n: number): string {
  if (!Number.isFinite(n) || n === 0) return "n/a";
  return n.toFixed(2);
}

function fmtKiB(kib: number): string {
  if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(2)} GiB`;
  if (kib >= 1024) return `${(kib / 1024).toFixed(1)} MiB`;
  return `${kib} KiB`;
}

function renderMarkdown(meta: any, steps: DensityStep[]): string {
  const lines: string[] = [];
  lines.push(`# view-density (Domain 3.2, multi-tenant scaling)`);
  lines.push("");
  lines.push(
    `Holds DB at ${meta.config.rows} rows; sweeps registered query count to characterise per-view memory and dispatch overhead.`,
  );
  lines.push("");
  lines.push(`| views | reg p50 | reg p95 | ingest p50 | ingest p95 | timeouts | RSS end | RSS max | per-added-view |`);
  lines.push(`| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |`);
  let prevRss: number | null = null;
  let prevViews: number | null = null;
  for (const s of steps) {
    const r = s.registration;
    const i = s.ingest;
    let perView = "n/a";
    if (prevRss !== null && prevViews !== null && s.views > prevViews) {
      const delta = s.rssEndKib - prevRss;
      const k = delta / (s.views - prevViews);
      perView = `${k.toFixed(0)} KiB/view`;
    }
    lines.push(
      `| ${s.views} | ${fmt(r.p50)} | ${fmt(r.p95)} | ${fmt(i.p50)} | ${fmt(i.p95)} | ${s.ingestTimeouts} | ${fmtKiB(s.rssEndKib)} | ${fmtKiB(s.rssMaxKib)} | ${perView} |`,
    );
    prevRss = s.rssEndKib;
    prevViews = s.views;
  }
  lines.push("");
  return lines.join("\n");
}
