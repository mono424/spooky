import { startSurrealDB, type SurrealHandle, type SurrealOptions } from "./surrealdb.js";
import { startScheduler, type SchedulerHandle, type SchedulerOptions } from "./scheduler.js";
import { startSsp, type SspHandle, type SspOptions } from "./ssp.js";
import { waitFor } from "../util/wait.js";
import { log } from "../util/log.js";

export interface Stack {
  surreal: SurrealHandle;
  scheduler: SchedulerHandle;
  ssp: SspHandle;
  /** Bearer secret shared by scheduler and SSP for authenticated routes. */
  authSecret: string;
  stop: () => Promise<void>;
}

export interface StackOptions {
  surreal?: SurrealOptions;
  scheduler?: Omit<SchedulerOptions, never>;
  ssp?: Omit<SspOptions, "schedulerUrl">;
  /** When true, child process stdout/stderr passes through. */
  verbose?: boolean;
}

export interface PreStack {
  surreal: SurrealHandle;
  authSecret: string;
  /**
   * Bring up the scheduler + SSP against this SurrealDB (which should already
   * be seeded). The SSP's `self_bootstrap` will run a `SELECT * FROM <table>`
   * for each user table at startup and load the rows into the circuit store,
   * so views registered afterwards will materialize against that data.
   */
  startBackend(): Promise<Stack>;
  /** Tear down SurrealDB only (use if startBackend hasn't been called). */
  stop(): Promise<void>;
}

/**
 * Two-phase orchestration: SurrealDB first, then (after the caller seeds it)
 * the scheduler + SSP.
 *
 * The split exists because the SSP populates its in-memory circuit store at
 * startup via `self_bootstrap` (one `SELECT * FROM <table>` per user table).
 * Any rows inserted *after* the SSP starts will not appear in the store, so
 * views registered later will materialize against an empty initial snapshot.
 * Seed the database between `startSurrealOnly` and `pre.startBackend()`.
 */
export async function startSurrealOnly(opts: StackOptions = {}): Promise<PreStack> {
  const verbose = opts.verbose ?? false;
  const surreal = await startSurrealDB({ ...opts.surreal, verbose });

  const authSecret =
    opts.scheduler?.env?.SPKY_AUTH_SECRET ??
    opts.ssp?.env?.SPKY_AUTH_SECRET ??
    process.env.SPKY_AUTH_SECRET ??
    "bench";

  let backendStarted = false;

  async function startBackend(): Promise<Stack> {
    if (backendStarted) throw new Error("startBackend called twice");
    backendStarted = true;

    let scheduler: SchedulerHandle | undefined;
    let ssp: SspHandle | undefined;

    try {
      scheduler = await startScheduler(surreal, {
        ...opts.scheduler,
        verbose,
        env: { ...(opts.scheduler?.env ?? {}), SPKY_AUTH_SECRET: authSecret },
      });
      ssp = await startSsp(surreal, {
        ...opts.ssp,
        schedulerUrl: scheduler.baseUrl,
        verbose,
        env: { ...(opts.ssp?.env ?? {}), SPKY_AUTH_SECRET: authSecret },
      });

      log.info("Waiting for scheduler to mark SSP as ready");
      await waitFor(
        async () => {
          const r = await fetch(`${scheduler!.baseUrl}/metrics`);
          if (!r.ok) return false;
          const m = (await r.json()) as { scheduler: { ready_ssps: number } };
          return m.scheduler.ready_ssps >= 1;
        },
        { timeoutMs: 60_000, intervalMs: 500, label: "scheduler.ready_ssps>=1" },
      );
      log.info("Stack ready");
    } catch (e) {
      if (ssp) await ssp.stop().catch(() => {});
      if (scheduler) await scheduler.stop().catch(() => {});
      await surreal.stop().catch(() => {});
      throw e;
    }

    const stop = async () => {
      await ssp!.stop().catch(() => {});
      await scheduler!.stop().catch(() => {});
      await surreal.stop().catch(() => {});
    };

    const onSignal = () => {
      stop()
        .catch(() => {})
        .finally(() => process.exit(130));
    };
    process.once("SIGINT", onSignal);
    process.once("SIGTERM", onSignal);

    return { surreal, scheduler: scheduler!, ssp: ssp!, authSecret, stop };
  }

  return {
    surreal,
    authSecret,
    startBackend,
    stop: async () => {
      await surreal.stop().catch(() => {});
    },
  };
}

/**
 * Convenience wrapper for the all-at-once order (SurrealDB → scheduler → SSP
 * with no seed in between). The SSP will bootstrap against an empty DB; only
 * useful for tests that don't depend on pre-existing rows.
 */
export async function startStack(opts: StackOptions = {}): Promise<Stack> {
  const pre = await startSurrealOnly(opts);
  return pre.startBackend();
}
