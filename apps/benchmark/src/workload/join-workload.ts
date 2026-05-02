import type { Surreal } from "surrealdb";
import { connectSurreal, type SurrealHandle } from "../stack/surrealdb.js";
import type { IngestRequest, ViewRegisterRequest } from "../drivers/scheduler-http.js";
import { log } from "../util/log.js";

/**
 * Seeds a controlled fan-out shape: 1 thread (id `thread:fan`) with K
 * comments referencing it. Used by the join-fanout suite to measure write
 * amplification when the single thread row is updated.
 */
export interface FanoutSeed {
  threads: number;
  comments: number;
}

export async function seedFanout(handle: SurrealHandle, fanout: number): Promise<FanoutSeed> {
  const db = await connectSurreal(handle);
  try {
    log.info(`Seeding fan-out: 1 thread, ${fanout} comments`);
    await db.query(
      `CREATE thread:fan SET title = "fan", content = "x", author = user:0, created_at = time::now();`,
    );
    await db.query(
      `CREATE user:0 SET username = "u0_seed", password = "x", created_at = time::now();`,
    );
    await insertCommentsForThread(db, "thread:fan", fanout);
    return { threads: 1, comments: fanout };
  } finally {
    await db.close();
  }
}

/**
 * Seeds a saturation shape: 100 small `thread` rows (active table) plus a
 * configurable number of `comment` rows referencing those threads (the
 * inactive lookup state). The benchmark sweeps the comment count.
 */
export interface SaturationSeed {
  threads: number;
  comments: number;
}

export async function seedSaturation(
  handle: SurrealHandle,
  threadCount: number,
  commentCount: number,
): Promise<SaturationSeed> {
  const db = await connectSurreal(handle);
  try {
    log.info(`Seeding saturation: ${threadCount} threads, ${commentCount} comments`);
    await db.query(
      `CREATE user:0 SET username = "u0_sat", password = "x", created_at = time::now();`,
    );
    for (let i = 0; i < threadCount; i += 500) {
      const end = Math.min(i + 500, threadCount);
      const rows: string[] = [];
      for (let j = i; j < end; j++) {
        rows.push(
          `{ id: thread:${j}, title: "t${j}", content: "c${j}", author: user:0, created_at: time::now() }`,
        );
      }
      await db.query(`INSERT INTO thread [${rows.join(",")}];`);
    }
    for (let i = 0; i < commentCount; i += 500) {
      const end = Math.min(i + 500, commentCount);
      const rows: string[] = [];
      for (let j = i; j < end; j++) {
        const tid = j % Math.max(1, threadCount);
        rows.push(
          `{ id: comment:${j}, thread: thread:${tid}, content: "c${j}", author: user:0, created_at: time::now() }`,
        );
      }
      await db.query(`INSERT INTO comment [${rows.join(",")}];`);
    }
    return { threads: threadCount, comments: commentCount };
  } finally {
    await db.close();
  }
}

async function insertCommentsForThread(db: Surreal, threadId: string, n: number): Promise<void> {
  for (let i = 0; i < n; i += 500) {
    const end = Math.min(i + 500, n);
    const rows: string[] = [];
    for (let j = i; j < end; j++) {
      rows.push(
        `{ id: comment:fan${j}, thread: ${threadId}, content: "c${j}", author: user:0, created_at: time::now() }`,
      );
    }
    await db.query(`INSERT INTO comment [${rows.join(",")}];`);
  }
}

/**
 * The canary view used by Domain 2 suites: a subquery-style "join", sp00ky's
 * SurQL → DBSP converter turns nested `(SELECT ... WHERE x = $parent.y) AS alias`
 * into a circuit that maintains the parent rows plus an inlined child set per
 * parent. (Standard SQL multi-table FROM is not parsed, the SurQL converter
 * doesn't emit the `Join` plan operator on `FROM a, b WHERE`. Verified by
 * testing: the SSP returns 400 Invalid Query Plan for SQL-equi-join syntax.)
 */
export const JOIN_CANARY_SURQL =
  `SELECT id, title, ` +
  `(SELECT id, content FROM comment WHERE thread = $parent.id) AS comments ` +
  `FROM thread`;

export function joinCanaryRequest(id: string, clientId: string): ViewRegisterRequest {
  return {
    id,
    surql: JOIN_CANARY_SURQL,
    clientId,
    ttl: "1h",
    lastActiveAt: new Date().toISOString(),
  };
}

/**
 * D2.1 trigger: insert a new comment that joins to `thread:fan`. The canary
 * view's row for thread:fan must re-evaluate its inlined `comments` subquery
 * over the existing K comments + 1 new one. Latency therefore scales with the
 * fan-out K. Each call generates a unique comment id.
 */
let fanoutCommentSeq = 0;
export function fanoutTriggerEvent(): IngestRequest {
  const seq = fanoutCommentSeq++;
  const id = `comment:fanmut${Date.now()}_${seq}`;
  return {
    table: "comment",
    op: "CREATE",
    id,
    record: {
      id,
      thread: "thread:fan",
      content: `mut${seq}`,
      author: "user:0",
      created_at: new Date().toISOString(),
    },
  };
}

/**
 * For D2.2: ingest a fresh thread:N row to test how heavy the inactive comment
 * table makes the active-side update path. The thread doesn't reference any
 * existing comments so write amplification stays at 1.
 */
export function saturationTriggerEvent(seq: number): IngestRequest {
  return {
    table: "thread",
    op: "CREATE",
    id: `thread:sat${seq}`,
    record: {
      id: `thread:sat${seq}`,
      title: `sat${seq}`,
      content: "x",
      author: "user:0",
      created_at: new Date().toISOString(),
    },
  };
}
