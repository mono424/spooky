import { startSurrealOnly, type Stack, type StackOptions } from "../stack/orchestrator.js";
import { seed, type SeedCounts, type SeedOptions } from "../workload/seed.js";
import { log } from "../util/log.js";

/**
 * Two-phase stack startup pattern shared by every suite: start SurrealDB,
 * seed data, then bring up scheduler+SSP so the SSP's self_bootstrap
 * (apps/ssp/src/lib.rs) actually loads the seeded rows into the circuit
 * store. Without this order, views register against an empty store.
 */
export interface SeededStack {
  stack: Stack;
  seedCounts: SeedCounts;
}

export async function startSeededStack(
  rows: number,
  stackOptions: StackOptions = {},
  seedOptions: SeedOptions = {},
): Promise<SeededStack> {
  const pre = await startSurrealOnly(stackOptions);
  let seedCounts: SeedCounts;
  let stack: Stack;
  try {
    seedCounts = await seed(pre.surreal, rows, seedOptions);
    stack = await pre.startBackend();
  } catch (e) {
    await pre.stop().catch(() => {});
    throw e;
  }
  return { stack, seedCounts };
}

/**
 * Same as startSeededStack but takes a custom seeder so suites with non-uniform
 * shapes (e.g. fan-out 1-to-K, saturation small-active-big-inactive) can
 * control distribution.
 */
export async function startCustomSeededStack<T>(
  seeder: (handle: import("../stack/surrealdb.js").SurrealHandle) => Promise<T>,
  stackOptions: StackOptions = {},
): Promise<{ stack: Stack; seedResult: T }> {
  const pre = await startSurrealOnly(stackOptions);
  let seedResult: T;
  let stack: Stack;
  try {
    seedResult = await seeder(pre.surreal);
    stack = await pre.startBackend();
  } catch (e) {
    await pre.stop().catch(() => {});
    throw e;
  }
  return { stack, seedResult };
}

/** Best-effort teardown that swallows errors. */
export async function safeStop(stack: Stack): Promise<void> {
  try {
    await stack.stop();
  } catch (e) {
    log.warn(`stack stop error: ${(e as Error).message}`);
  }
}
