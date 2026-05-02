import path from "node:path";
import fs from "node:fs";
import net from "node:net";
import { execa, type ResultPromise } from "execa";
import { waitForHttp } from "../util/wait.js";
import { log } from "../util/log.js";
import type { SurrealHandle } from "./surrealdb.js";

export interface SspHandle {
  baseUrl: string;
  sspId: string;
  pid: number | undefined;
  child: ResultPromise;
  stop: () => Promise<void>;
  /**
   * Stop the SSP and start a fresh one with the same options. Used by the
   * backlog suite to simulate an outage. The returned handle replaces this
   * one (the original is dead afterwards), caller must update its
   * reference.
   */
  restart: () => Promise<SspHandle>;
}

export interface SspOptions {
  binPath?: string;
  repoRoot?: string;
  port?: number;
  schedulerUrl: string;
  sspId?: string;
  verbose?: boolean;
  env?: Record<string, string>;
  workDir?: string;
}

const DEFAULT_PORT = 8667;

function repoRootFromHere(): string {
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

export async function startSsp(
  surreal: SurrealHandle,
  opts: SspOptions,
): Promise<SspHandle> {
  const port = opts.port ?? DEFAULT_PORT;
  if (!(await isPortFree(port))) {
    throw new Error(`SSP port ${port} in use. Pass --ssp-port or stop the conflicting process.`);
  }
  const repoRoot = opts.repoRoot ?? repoRootFromHere();
  const sspId = opts.sspId ?? `bench-ssp-${Date.now()}`;
  const workDir = opts.workDir ?? path.join(process.cwd(), `.bench-ssp-${Date.now()}`);
  fs.mkdirSync(workDir, { recursive: true });

  const baseUrl = `http://127.0.0.1:${port}`;

  const env: Record<string, string> = {
    ...process.env,
    SPKY_SSP_LISTEN_ADDR: `127.0.0.1:${port}`,
    SPKY_SSP_ADVERTISE_ADDR: `127.0.0.1:${port}`,
    SPKY_DB_WS: surreal.wsUrl,
    SPKY_DB_NS: surreal.namespace,
    SPKY_DB_NAME: surreal.database,
    SPKY_DB_USER: surreal.username,
    SPKY_DB_PASS: surreal.password,
    SPKY_SCHEDULER_URL: opts.schedulerUrl,
    SPKY_SSP_ID: sspId,
    // Must match the scheduler's SPKY_AUTH_SECRET, both read the same env var
    // and the SSP expects `Authorization: Bearer <secret>` on authenticated
    // routes (apps/ssp/src/lib.rs auth_middleware).
    SPKY_AUTH_SECRET: opts.env?.SPKY_AUTH_SECRET ?? process.env.SPKY_AUTH_SECRET ?? "bench",
    RUST_LOG: process.env.RUST_LOG ?? "warn,ssp_server=info,ssp=info",
    ...(opts.env ?? {}),
  };

  const workspaceBin = path.join(repoRoot, "target/release/ssp-server");
  const perAppBin = path.join(repoRoot, "apps/ssp/target/release/ssp-server");
  const candidate = opts.binPath ?? (fs.existsSync(workspaceBin) ? workspaceBin : perAppBin);
  const useCargo = !fs.existsSync(candidate);
  const cmd = useCargo ? "cargo" : candidate;
  const args = useCargo
    ? ["run", "--release", "--manifest-path", path.join(repoRoot, "apps/ssp/Cargo.toml")]
    : [];

  log.info(`Starting SSP (${useCargo ? "cargo run" : candidate}) on ${baseUrl}, id=${sspId}`);

  const child = execa(cmd, args, {
    cwd: workDir,
    env,
    stdout: opts.verbose ? "inherit" : "ignore",
    stderr: opts.verbose ? "inherit" : "pipe",
    reject: false,
    forceKillAfterDelay: 5_000,
  });

  let stderrTail = "";
  if (!opts.verbose && child.stderr) {
    child.stderr.on("data", (chunk: Buffer) => {
      stderrTail = (stderrTail + chunk.toString()).slice(-4000);
    });
  }

  let exited = false;
  child.then((res) => {
    exited = true;
    if (res.exitCode && res.exitCode !== 0) {
      log.error(`SSP exited (code=${res.exitCode}). Tail:\n${stderrTail}`);
    }
  });

  // SSP exposes a public /health endpoint (apps/ssp/src/lib.rs router:
  // public Router::new().route("/health", ...)). The SSP returns 200 only
  // after `self_bootstrap` finishes loading all user tables into its circuit
  // store. For multi-GB DBs that bootstrap is dominated by JSON ser/de of
  // the full result set returned from the scheduler proxy, so we allow up
  // to 10 min before declaring the SSP unreachable.
  await waitForHttp(`${baseUrl}/health`, {
    timeoutMs: 600_000,
    expectStatus: 200,
    label: "ssp /health",
  }).catch(async (e) => {
    if (exited) {
      throw new Error(
        `SSP exited before readiness. Stderr tail:\n${stderrTail}\n\nOriginal: ${(e as Error).message}`,
      );
    }
    throw e;
  });

  log.info(`SSP up at ${baseUrl}`);

  const stop = async () => {
    log.info("Stopping SSP");
    try {
      child.kill("SIGTERM");
    } catch { /* already gone */ }
    try {
      await child;
    } catch { /* swallow */ }
  };

  return {
    baseUrl,
    sspId,
    pid: child.pid,
    child,
    stop,
    restart: async () => {
      await stop();
      // Use a FRESH ssp_id on restart. Reusing the original id makes the
      // scheduler treat the restart as a re-registration of the same SSP,
      // which triggers the "scheduler requested re-bootstrap" exit path
      // (apps/ssp/src/lib.rs:579) since the scheduler still holds state from
      // the previous instance. A fresh id makes the scheduler treat it as a
      // brand-new SSP and bootstrap it cleanly.
      const restartedOpts = {
        ...opts,
        sspId: `bench-ssp-restart-${Date.now()}`,
      };
      return startSsp(surreal, restartedOpts);
    },
  };
}
