import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { SspClient } from "../drivers/ssp-http.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { startSeededStack, safeStop } from "./common.js";
import { subqueryView } from "../workload/subquery-workload.js";
import { generateIngests } from "../workload/ingest.js";
import { log } from "../util/log.js";
import { nowMs } from "../util/time.js";

interface DepthResult {
  depth: 1 | 2 | 3;
  /** True if SSP returned a usable view (some plans may not be supported). */
  registered: boolean;
  /** Why registration failed if it did. */
  registerError?: string;
  /** End-to-end propagation latency for the depth-K canary. */
  latency: LatencySummary;
  /** Final canary cache_size. */
  finalCacheSize: number | null;
}

export interface SubqueryDepthArgs {
  rows: number;
  ingestEvents: number;
  /** Padding bytes per comment row. 0 = baseline ~111B/row. */
  rowBytes?: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runSubqueryDepth(args: SubqueryDepthArgs): Promise<void> {
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const env = envSnapshot();
  const sha = await gitSha();

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
  log.info(`seeded ${(seedCounts.approxBytes / 1024 / 1024).toFixed(1)} MiB of data`);
  try {
    const client = new SchedulerClient(stack.scheduler.baseUrl);
    const ssp = new SspClient(stack.ssp.baseUrl, stack.authSecret);

    const depths: (1 | 2 | 3)[] = [1, 2, 3];
    const canaryIds = new Map<number, string>();
    const lastHashes = new Map<number, string>();
    const registerErrors = new Map<number, string>();
    const cacheSizes = new Map<number, number | null>();

    for (const d of depths) {
      const id = `bench:subq:${d}:${randomUUID()}`;
      const view = subqueryView(d, id, `subq-${d}`);
      try {
        await client.registerView(view);
        const v = await ssp.getDebugView(id);
        canaryIds.set(d, id);
        lastHashes.set(d, v?.last_hash ?? "");
        cacheSizes.set(d, v?.cache_size ?? null);
        log.info(`registered depth-${d}, initial cache_size=${v?.cache_size ?? "?"}`);
      } catch (e) {
        registerErrors.set(d, (e as Error).message);
        log.warn(`depth-${d} register failed: ${(e as Error).message}`);
      }
    }

    const recorders = new Map<number, LatencyRecorder>();
    for (const d of depths) recorders.set(d, new LatencyRecorder());

    const ingests = generateIngests(
      args.ingestEvents,
      { users: Math.max(1, seedCounts.users), threads: Math.max(1, seedCounts.threads) },
      8_000_000,
    );

    for (const ev of ingests) {
      const start = nowMs();
      await client.ingest(ev);
      // Poll each registered depth's canary for hash advance, in parallel.
      await Promise.all(
        depths
          .filter((d) => canaryIds.has(d))
          .map(async (d) => {
            const canary = canaryIds.get(d)!;
            const prev = lastHashes.get(d)!;
            const deadline = nowMs() + 5000;
            while (nowMs() < deadline) {
              const v = await ssp.getDebugView(canary).catch(() => null);
              if (v && v.last_hash && v.last_hash !== prev) {
                lastHashes.set(d, v.last_hash);
                cacheSizes.set(d, v.cache_size);
                recorders.get(d)!.record(nowMs() - start);
                return;
              }
              await new Promise((r) => setImmediate(r));
            }
            // Timed out, record a max-window sample so it shows in tail stats.
            recorders.get(d)!.record(5000);
          }),
      );
    }

    const results: DepthResult[] = depths.map((d) => ({
      depth: d,
      registered: canaryIds.has(d),
      registerError: registerErrors.get(d),
      latency: recorders.get(d)!.summarize(),
      finalCacheSize: cacheSizes.get(d) ?? null,
    }));

    fs.mkdirSync(args.outDir, { recursive: true });
    const meta = {
      suite: "subquery-depth",
      domain: "1.3 pipeline depth (reframed: SurQL subquery depth)",
      startedAt,
      finishedAt: new Date().toISOString(),
      durationSec: (Date.now() - t0) / 1000,
      config: { rows: args.rows, ingestEvents: args.ingestEvents },
      env: { ...env, gitSha: sha },
    };
    fs.writeFileSync(
      path.join(args.outDir, "samples.json"),
      JSON.stringify({ meta, results }, null, 2),
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
  return n.toFixed(2);
}

function renderMarkdown(meta: any, results: DepthResult[]): string {
  const lines: string[] = [];
  lines.push(`# subquery-depth (Domain 1.3, pipeline depth, reframed)`);
  lines.push("");
  lines.push(
    `sp00ky doesn't allow registering a view that reads from another registered view. This suite uses SurQL inlined subqueries to exercise the same "compounding propagation" idea: depth-1 is a flat \`SELECT\`, depth-2 inlines a child query, depth-3 inlines two levels of child queries.`,
  );
  lines.push("");
  lines.push(`| depth | registered | initial cache_size | p50 (ms) | p95 (ms) | p99 (ms) | max |`);
  lines.push(`| ---: | :---: | ---: | ---: | ---: | ---: | ---: |`);
  for (const r of results) {
    if (!r.registered) {
      lines.push(
        `| ${r.depth} | ✗ | n/a | n/a | n/a | n/a | (register failed: ${r.registerError ?? "?"}) |`,
      );
      continue;
    }
    const l = r.latency;
    lines.push(
      `| ${r.depth} | ✓ | ${r.finalCacheSize ?? "?"} | ${fmt(l.p50)} | ${fmt(l.p95)} | ${fmt(l.p99)} | ${fmt(l.max)} |`,
    );
  }
  lines.push("");
  return lines.join("\n");
}
