import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SchedulerClient } from "../drivers/scheduler-http.js";
import { SspClient } from "../drivers/ssp-http.js";
import { envSnapshot, gitSha } from "../metrics/report.js";
import { measureAmplification, type AmplificationSample } from "../metrics/write-amplification.js";
import { joinCanaryRequest, fanoutTriggerEvent, seedFanout } from "../workload/join-workload.js";
import { startCustomSeededStack, safeStop } from "./common.js";
import { log } from "../util/log.js";

interface FanoutStep {
  fanout: number;
  registered: boolean;
  initialCacheSize: number | null;
  registerError?: string;
  samples: AmplificationSample[];
  /** Mean amplification (output records emitted per input event). */
  meanAmplification: number;
  /** Mean settle time. */
  meanSettleMs: number;
}

export interface JoinFanoutArgs {
  fanouts: number[];
  iterationsPerStep: number;
  outDir: string;
  smoke?: boolean;
  surrealImage?: string;
  modulesDir?: string;
  schemaPath?: string;
  schedulerBin?: string;
  sspBin?: string;
  verbose?: boolean;
}

export async function runJoinFanout(args: JoinFanoutArgs): Promise<void> {
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const env = envSnapshot();
  const sha = await gitSha();

  const steps: FanoutStep[] = [];
  for (const fanout of args.fanouts) {
    log.step(`join-fanout: fanout=${fanout}`);
    const { stack, seedResult } = await startCustomSeededStack(
      (h) => seedFanout(h, fanout),
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
    void seedResult;

    try {
      const client = new SchedulerClient(stack.scheduler.baseUrl);
      const ssp = new SspClient(stack.ssp.baseUrl, stack.authSecret);
      const canaryId = `bench:join:${fanout}:${randomUUID()}`;

      let registered = false;
      let registerError: string | undefined;
      let initialCacheSize: number | null = null;
      try {
        await client.registerView(joinCanaryRequest(canaryId, `fanout-${fanout}`));
        const v = await ssp.getDebugView(canaryId);
        registered = !!v;
        initialCacheSize = v?.cache_size ?? null;
        log.info(`canary registered, initial cache_size=${initialCacheSize}`);
      } catch (e) {
        registerError = (e as Error).message;
        log.warn(`canary register failed: ${registerError}`);
      }

      const samples: AmplificationSample[] = [];
      if (registered) {
        for (let i = 0; i < args.iterationsPerStep; i++) {
          const sample = await measureAmplification(
            ssp,
            canaryId,
            async () => {
              await client.ingest(fanoutTriggerEvent());
            },
            { timeoutMs: 15_000, stableWindowMs: 200 },
          );
          samples.push(sample);
        }
      }

      const meanAmplification =
        samples.length === 0 ? 0 : samples.reduce((a, s) => a + Math.abs(s.delta), 0) / samples.length;
      const meanSettleMs =
        samples.length === 0 ? 0 : samples.reduce((a, s) => a + s.settleMs, 0) / samples.length;

      steps.push({
        fanout,
        registered,
        initialCacheSize,
        registerError,
        samples,
        meanAmplification,
        meanSettleMs,
      });
      log.info(
        `fanout=${fanout} mean amp=${meanAmplification.toFixed(1)} settle p50=${meanSettleMs.toFixed(2)}ms`,
      );
    } finally {
      await safeStop(stack);
    }
  }

  fs.mkdirSync(args.outDir, { recursive: true });
  const meta = {
    suite: "join-fanout",
    domain: "2.1 join fan-out (write amplification)",
    startedAt,
    finishedAt: new Date().toISOString(),
    durationSec: (Date.now() - t0) / 1000,
    config: { fanouts: args.fanouts, iterationsPerStep: args.iterationsPerStep },
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

function renderMarkdown(meta: any, steps: FanoutStep[]): string {
  const lines: string[] = [];
  lines.push(`# join-fanout (Domain 2.1, subquery-join cascade cost)`);
  lines.push("");
  lines.push(
    `Each step seeds 1 thread + K comments referencing it, registers a subquery-join canary (\`SELECT id, title, (SELECT id, content FROM comment WHERE thread = $parent.id) AS comments FROM thread\`), then issues \`${meta.config.iterationsPerStep}\` CREATE comment events that match thread:fan. The SSP must re-evaluate the parent row's inlined subquery after each child insert. Settle time is wall-clock from \`POST /ingest\` to the canary view's content stabilising for ≥200 ms.`,
  );
  lines.push("");
  lines.push(
    `Note: \`cache_size\` here counts **parent rows** in the materialized view (threads), not child rows. The canary always has 1 parent (thread:fan), so cache_size stays at 1 and the column is not a meaningful "amplification" signal in this shape. Settle time is the right cost metric for subquery-style joins; an inlined SELECT scaling with K children should show up there.`,
  );
  lines.push("");
  lines.push(`| fanout | registered | parent rows in cache | mean settle (ms) |`);
  lines.push(`| ---: | :---: | ---: | ---: |`);
  for (const s of steps) {
    if (!s.registered) {
      lines.push(`| ${s.fanout} | ✗ | n/a | n/a (${s.registerError ?? "?"}) |`);
      continue;
    }
    lines.push(
      `| ${s.fanout} | ✓ | ${s.initialCacheSize ?? "?"} | ${s.meanSettleMs.toFixed(2)} |`,
    );
  }
  lines.push("");
  return lines.join("\n");
}
