import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { SspClient } from "../drivers/ssp-http.js";
import { LatencyRecorder, type LatencySummary } from "../metrics/latency.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { joinCanaryRequest, saturationTriggerEvent, seedSaturation } from "../workload/join-workload.js";
import { startCustomSeededStack, safeStop } from "./common.js";
import { log } from "../util/log.js";
import { nowMs } from "../util/time.js";

interface SaturationStep {
  inactiveSize: number;
  activeSize: number;
  registered: boolean;
  registerError?: string;
  initialCacheSize: number | null;
  ingestLatency: LatencySummary;
  timeouts: number;
}

export interface JoinSaturationArgs {
  inactiveSizes: number[];
  activeSize: number;
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

export async function runJoinSaturation(args: JoinSaturationArgs): Promise<void> {
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const env = envSnapshot();
  const sha = await gitSha();

  const steps: SaturationStep[] = [];
  for (const inactive of args.inactiveSizes) {
    log.step(`join-saturation: active=${args.activeSize} inactive=${inactive}`);
    const { stack } = await startCustomSeededStack(
      (h) => seedSaturation(h, args.activeSize, inactive),
      {
        verbose: args.verbose,
        surreal: {
          image: args.surrealImage,
          modulesDir: args.modulesDir,
          schemaPath: args.schemaPath,
        },
        scheduler: { binPath: args.schedulerBin },
        ssp: { binPath: args.sspBin },
      },
    );

    try {
      const client = new SchedulerClient(stack.scheduler.baseUrl);
      const ssp = new SspClient(stack.ssp.baseUrl, stack.authSecret);
      const canaryId = `bench:sat:${inactive}:${randomUUID()}`;

      let registered = false;
      let registerError: string | undefined;
      let initialCacheSize: number | null = null;
      try {
        await client.registerView(joinCanaryRequest(canaryId, `sat-${inactive}`));
        const v = await ssp.getDebugView(canaryId);
        registered = !!v;
        initialCacheSize = v?.cache_size ?? null;
      } catch (e) {
        registerError = (e as Error).message;
      }

      const rec = new LatencyRecorder();
      let timeouts = 0;
      let lastHash = registered ? (await ssp.getDebugView(canaryId))?.last_hash ?? "" : "";

      if (registered) {
        for (let i = 0; i < args.ingestEvents; i++) {
          const ev = saturationTriggerEvent(20_000_000 + i);
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
          rec.record(nowMs() - start);
          if (!advanced) timeouts++;
        }
      }

      steps.push({
        inactiveSize: inactive,
        activeSize: args.activeSize,
        registered,
        registerError,
        initialCacheSize,
        ingestLatency: rec.summarize(),
        timeouts,
      });
    } finally {
      await safeStop(stack);
    }
  }

  fs.mkdirSync(args.outDir, { recursive: true });
  const meta = {
    suite: "join-saturation",
    domain: "2.2 join state saturation",
    startedAt,
    finishedAt: new Date().toISOString(),
    durationSec: (Date.now() - t0) / 1000,
    config: {
      inactiveSizes: args.inactiveSizes,
      activeSize: args.activeSize,
      ingestEvents: args.ingestEvents,
    },
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

function renderMarkdown(meta: any, steps: SaturationStep[]): string {
  const lines: string[] = [];
  lines.push(`# join-saturation (Domain 2.2, inactive-side scaling)`);
  lines.push("");
  lines.push(
    `Active table (\`thread\`) holds ${meta.config.activeSize} rows; the inactive lookup table (\`comment\`) is swept. Each step ingests fresh \`thread\` rows (no children) and measures end-to-end propagation latency through the equi-join canary. The hypothesis: a larger inactive lookup state should NOT slow down activity on the small active side, because DBSP only walks indexed keys touched by the delta.`,
  );
  lines.push("");
  lines.push(`Capped at 10k inactive rows: scheduler bootstrap is O(N) per record (\`apps/scheduler/src/replica.rs\`).`);
  lines.push("");
  lines.push(`| inactive size | registered | initial cache | timeouts | p50 (ms) | p95 | p99 | max |`);
  lines.push(`| ---: | :---: | ---: | ---: | ---: | ---: | ---: | ---: |`);
  for (const s of steps) {
    if (!s.registered) {
      lines.push(
        `| ${s.inactiveSize} | ✗ | n/a | n/a | n/a | n/a | n/a | (register failed: ${s.registerError ?? "?"}) |`,
      );
      continue;
    }
    const l = s.ingestLatency;
    lines.push(
      `| ${s.inactiveSize} | ✓ | ${s.initialCacheSize ?? "?"} | ${s.timeouts} | ${fmt(l.p50)} | ${fmt(l.p95)} | ${fmt(l.p99)} | ${fmt(l.max)} |`,
    );
  }
  lines.push("");
  return lines.join("\n");
}
