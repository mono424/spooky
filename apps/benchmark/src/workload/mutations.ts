import type { IngestRequest } from "../drivers/scheduler-http.js";

export interface MutationMix {
  /** Fraction of events that are CREATE (0–1). */
  create: number;
  /** Fraction of events that are UPDATE (0–1). */
  update: number;
  /** Fraction of events that are DELETE (0–1). */
  delete: number;
}

export interface GeneratedMutation extends IngestRequest {
  /** Op tag preserved from generation so the runner can record per-op latencies. */
  _op: "CREATE" | "UPDATE" | "DELETE";
}

export interface MutationGenerator {
  /** Total events that will be emitted. */
  size: number;
  /** Pull the next mutation. Throws past `size`. */
  next(): GeneratedMutation;
}

/**
 * Builds a deterministic mutation generator for the `comment` table that
 * targets only previously-known ids on UPDATE/DELETE so the SSP's circuit
 * has a row to mutate.
 *
 * `existingIds` is the pool of comment ids that are already in the database
 * (typically pre-seeded). UPDATE picks one at random (round-robin); DELETE
 * pops one off so subsequent mutations don't try to delete it again.
 */
export function makeMutationGenerator(
  count: number,
  mix: MutationMix,
  existingIds: string[],
  seedScale: { users: number; threads: number },
): MutationGenerator {
  const total = mix.create + mix.update + mix.delete;
  if (Math.abs(total - 1) > 1e-6) {
    throw new Error(`mutation mix must sum to 1.0, got ${total}`);
  }

  // Schedule emitted ops up front so the mix is exact, not stochastic.
  const ops: ("CREATE" | "UPDATE" | "DELETE")[] = [];
  const numCreate = Math.round(count * mix.create);
  const numUpdate = Math.round(count * mix.update);
  const numDelete = count - numCreate - numUpdate;
  for (let i = 0; i < numCreate; i++) ops.push("CREATE");
  for (let i = 0; i < numUpdate; i++) ops.push("UPDATE");
  for (let i = 0; i < numDelete; i++) ops.push("DELETE");
  // Interleave deterministically so a 70/20/10 mix doesn't run as three blocks.
  for (let i = ops.length - 1; i > 0; i--) {
    const j = (i * 2654435761) % (i + 1);
    [ops[i], ops[j]] = [ops[j]!, ops[i]!];
  }

  const live: string[] = [...existingIds];
  let cursor = 0;
  let createCounter = 0;

  return {
    size: count,
    next(): GeneratedMutation {
      if (cursor >= ops.length) throw new Error("generator exhausted");
      const op = ops[cursor]!;
      cursor++;
      const uid = (cursor * 31 + createCounter) % Math.max(1, seedScale.users);
      const tid = (cursor * 17) % Math.max(1, seedScale.threads);

      if (op === "CREATE") {
        const id = `comment:mut${1_000_000 + createCounter++}`;
        live.push(id);
        return {
          _op: "CREATE",
          table: "comment",
          op: "CREATE",
          id,
          record: {
            id,
            thread: `thread:${tid}`,
            content: `mut${cursor}`,
            author: `user:${uid}`,
            created_at: new Date().toISOString(),
          },
        };
      }
      if (op === "UPDATE") {
        if (live.length === 0) {
          // Fall back to CREATE if we ran out, shouldn't happen if existingIds is non-empty.
          return this.next();
        }
        const id = live[cursor % live.length]!;
        return {
          _op: "UPDATE",
          table: "comment",
          op: "UPDATE",
          id,
          record: {
            id,
            thread: `thread:${tid}`,
            content: `upd${cursor}`,
            author: `user:${uid}`,
            created_at: new Date().toISOString(),
          },
        };
      }
      // DELETE
      if (live.length === 0) return this.next();
      const id = live.shift()!;
      return {
        _op: "DELETE",
        table: "comment",
        op: "DELETE",
        id,
        record: { id },
      };
    },
  };
}
