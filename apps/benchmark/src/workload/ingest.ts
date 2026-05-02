import type { IngestRequest } from "../drivers/scheduler-http.js";

export interface GeneratedIngest extends IngestRequest {}

/**
 * Generates `count` ingest events. Mix is biased toward CREATE comments
 * since that's the most write-heavy table in the schema. The records
 * reference user/thread ids in the seeded range modulo `seedScale`.
 */
export function generateIngests(
  count: number,
  seedScale: { users: number; threads: number },
  startOffset: number = 1_000_000,
): GeneratedIngest[] {
  const users = Math.max(1, seedScale.users);
  const threads = Math.max(1, seedScale.threads);
  const out: GeneratedIngest[] = [];
  for (let i = 0; i < count; i++) {
    const j = startOffset + i;
    const author = j % users;
    const thread = j % threads;
    out.push({
      table: "comment",
      op: "CREATE",
      id: `comment:bench${j}`,
      record: {
        id: `comment:bench${j}`,
        thread: `thread:${thread}`,
        content: `bench${j}`,
        author: `user:${author}`,
        created_at: new Date().toISOString(),
      },
    });
  }
  return out;
}
