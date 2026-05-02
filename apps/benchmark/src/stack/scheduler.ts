import path from "node:path";
import fs from "node:fs";
import net from "node:net";
import { execa, type ResultPromise } from "execa";
import { waitForHttp } from "../util/wait.js";
import { log } from "../util/log.js";
import type { SurrealHandle } from "./surrealdb.js";

export interface SchedulerHandle {
  baseUrl: string;
  pid: number | undefined;
  child: ResultPromise;
  stop: () => Promise<void>;
}

export interface SchedulerOptions {
  /** Path to a prebuilt scheduler binary; if absent we use cargo run --release. */
  binPath?: string;
  /** Repo root, defaults to apps/benchmark/../.. */
  repoRoot?: string;
  /** Listen port. Default 9667. */
  port?: number;
  /** Verbose: pipe scheduler stdout to ours. */
  verbose?: boolean;
  /** Extra env */
  env?: Record<string, string>;
  /** Working directory for state (replica db, wal). Default = a temp dir. */
  workDir?: string;
}

const DEFAULT_PORT = 9667;

function repoRootFromHere(): string {
  // .../apps/benchmark/dist/stack/scheduler.js → repo root is ../../../..
  return path.resolve(import.meta.dirname, "../../../..");
}

async function isPortFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.once("error", () => resolve(false));
    s.once("listening", () => s.close(() => resolve(true)));
    s.listen(port, "127.0.0.1");
  });
}

export async function startScheduler(
  surreal: SurrealHandle,
  opts: SchedulerOptions = {},
): Promise<SchedulerHandle> {
  const port = opts.port ?? DEFAULT_PORT;
  if (!(await isPortFree(port))) {
    throw new Error(
      `Scheduler port ${port} is in use. Pass --scheduler-port or stop the conflicting process.`,
    );
  }
  const repoRoot = opts.repoRoot ?? repoRootFromHere();
  const workDir = opts.workDir ?? path.join(process.cwd(), `.bench-scheduler-${Date.now()}`);
  fs.mkdirSync(workDir, { recursive: true });

  const env: Record<string, string> = {
    ...process.env,
    SPKY_DB_WS: surreal.wsUrl,
    SPKY_DB_NS: surreal.namespace,
    SPKY_DB_NAME: surreal.database,
    SPKY_DB_USER: surreal.username,
    SPKY_DB_PASS: surreal.password,
    // Scheduler must send `Authorization: Bearer <secret>` to the SSP, and the
    // SSP requires the header on authenticated routes. Both processes read the
    // same env var; mismatch → SSP returns 401.
    SPKY_AUTH_SECRET: opts.env?.SPKY_AUTH_SECRET ?? process.env.SPKY_AUTH_SECRET ?? "bench",
    RUST_LOG: process.env.RUST_LOG ?? "warn,scheduler=info",
    ...(opts.env ?? {}),
  };

  const baseUrl = `http://127.0.0.1:${port}`;

  // Default: prefer workspace target (the canonical Cargo workspace output),
  // fall back to per-app target, finally fall back to `cargo run --release`.
  const workspaceBin = path.join(repoRoot, "target/release/scheduler");
  const perAppBin = path.join(repoRoot, "apps/scheduler/target/release/scheduler");
  const candidate = opts.binPath ?? (fs.existsSync(workspaceBin) ? workspaceBin : perAppBin);

  const useCargo = !fs.existsSync(candidate);
  const cmd = useCargo ? "cargo" : candidate;
  const args = useCargo
    ? ["run", "--release", "--manifest-path", path.join(repoRoot, "apps/scheduler/Cargo.toml")]
    : [];

  log.info(`Starting scheduler (${useCargo ? "cargo run" : candidate}) on ${baseUrl}, workDir=${workDir}`);

  const child = execa(cmd, args, {
    cwd: workDir,
    env,
    stdout: opts.verbose ? "inherit" : "ignore",
    stderr: opts.verbose ? "inherit" : "pipe",
    reject: false,
    forceKillAfterDelay: 5_000,
  });

  // Buffer last bit of stderr in case startup fails
  let stderrTail = "";
  if (!opts.verbose && child.stderr) {
    child.stderr.on("data", (chunk: Buffer) => {
      stderrTail = (stderrTail + chunk.toString()).slice(-4000);
    });
  }

  // If the scheduler exits before becoming ready, surface its stderr.
  let exited = false;
  child.then((res) => {
    exited = true;
    if (res.exitCode && res.exitCode !== 0) {
      log.error(`Scheduler exited (code=${res.exitCode}). Tail:\n${stderrTail}`);
    }
  });

  try {
    // The scheduler clones SurrealDB into its local replica during bootstrap;
    // for large DBs this can take minutes. Be generous.
    await waitForHttp(`${baseUrl}/health/ready`, {
      timeoutMs: 600_000,
      label: "scheduler /health/ready",
    });
  } catch (e) {
    if (exited) {
      throw new Error(
        `Scheduler exited before readiness. Stderr tail:\n${stderrTail}\n\nOriginal: ${(e as Error).message}`,
      );
    }
    throw e;
  }

  log.info(`Scheduler ready at ${baseUrl}`);

  return {
    baseUrl,
    pid: child.pid,
    child,
    stop: async () => {
      log.info("Stopping scheduler");
      try {
        child.kill("SIGTERM");
      } catch { /* already gone */ }
      try {
        await child;
      } catch { /* swallow */ }
    },
  };
}
