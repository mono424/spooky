import type { Surreal } from "surrealdb";
import { connectSurreal, type SurrealHandle } from "../stack/surrealdb.js";
import { log } from "../util/log.js";

/**
 * Seeds the test schema (user/thread/comment) with `targetRows` total rows.
 * Distribution: 10% users, 30% threads, 60% comments. Uses batched
 * `INSERT INTO ...` statements over a single connection.
 *
 * `commentPadBytes` adds N bytes of deterministic padding to every comment's
 * `content` field, used to push total DB volume into MB or GB territory
 * without growing the row count (the scheduler bootstrap is per-record
 * sequential, so growing rows is the only practical way to hit big DBs).
 *
 * Returns counts per table plus the approximate total payload bytes seeded.
 */
export interface SeedCounts {
  users: number;
  threads: number;
  comments: number;
  /** Approximate total JSON bytes of seeded data. */
  approxBytes: number;
}

export interface SeedOptions {
  /** Bytes of padding to attach to each comment's `content` field. Default 0. */
  commentPadBytes?: number;
}

export async function seed(
  handle: SurrealHandle,
  targetRows: number,
  opts: SeedOptions = {},
): Promise<SeedCounts> {
  if (targetRows < 0) throw new Error("targetRows must be >= 0");
  const commentPadBytes = Math.max(0, opts.commentPadBytes ?? 0);
  if (targetRows === 0) return { users: 0, threads: 0, comments: 0, approxBytes: 0 };

  // Always at least 1 user, 1 thread when targetRows > 0 so fkey-style fields resolve.
  const users = Math.max(1, Math.floor(targetRows * 0.1));
  const threads = Math.max(1, Math.floor(targetRows * 0.3));
  const comments = Math.max(0, targetRows - users - threads);

  const db = await connectSurreal(handle);
  try {
    const padNote = commentPadBytes > 0 ? `, pad ${commentPadBytes}B/comment` : "";
    log.info(`Seeding ${users} users, ${threads} threads, ${comments} comments${padNote}`);

    await insertUsers(db, users);
    await insertThreads(db, threads, users);
    await insertComments(db, comments, users, threads, commentPadBytes);

    // Rough JSON byte estimate matching the per-row sizes the seeder writes.
    const approxBytes =
      users * 91 + threads * 102 + comments * (111 + commentPadBytes);
    return { users, threads, comments, approxBytes };
  } finally {
    await db.close();
  }
}

const USER_BATCH = 500;
const THREAD_BATCH = 500;

async function insertUsers(db: Surreal, n: number): Promise<void> {
  for (let i = 0; i < n; i += USER_BATCH) {
    const end = Math.min(i + USER_BATCH, n);
    const rows: string[] = [];
    for (let j = i; j < end; j++) {
      rows.push(
        `{ id: user:${j}, username: "u${j}_${Math.random().toString(36).slice(2, 8)}", password: "x", created_at: time::now() }`,
      );
    }
    await db.query(`INSERT INTO user [${rows.join(",")}];`);
  }
}

async function insertThreads(db: Surreal, n: number, users: number): Promise<void> {
  for (let i = 0; i < n; i += THREAD_BATCH) {
    const end = Math.min(i + THREAD_BATCH, n);
    const rows: string[] = [];
    for (let j = i; j < end; j++) {
      const authorIx = j % users;
      rows.push(
        `{ id: thread:${j}, title: "t${j}", content: "c${j}", author: user:${authorIx}, created_at: time::now() }`,
      );
    }
    await db.query(`INSERT INTO thread [${rows.join(",")}];`);
  }
}

/**
 * Insert comments with optional content padding. Larger `padBytes` makes each
 * row larger and the total DB volume scale proportionally. Batch size is
 * tuned down when rows are big so we don't blow the SurrealDB query size limit.
 */
async function insertComments(
  db: Surreal,
  n: number,
  users: number,
  threads: number,
  padBytes: number,
): Promise<void> {
  // Aim for batches under ~2 MiB. Each comment is ~111 + padBytes bytes.
  const targetBatchBytes = 2 * 1024 * 1024;
  const perRow = 111 + Math.max(0, padBytes);
  const batch = Math.max(1, Math.min(500, Math.floor(targetBatchBytes / perRow)));

  // Build the padding string once. Use a deterministic ASCII filler to keep
  // the JSON ASCII-safe and avoid any per-row randomness influencing perf.
  const padString = padBytes > 0 ? "x".repeat(padBytes) : "";

  for (let i = 0; i < n; i += batch) {
    const end = Math.min(i + batch, n);
    const rows: string[] = [];
    for (let j = i; j < end; j++) {
      const author = j % users;
      const thread = j % threads;
      const base = `msg${j}`;
      const content = padBytes > 0 ? `${base}${padString}` : base;
      rows.push(
        `{ id: comment:${j}, thread: thread:${thread}, content: "${content}", author: user:${author}, created_at: time::now() }`,
      );
    }
    await db.query(`INSERT INTO comment [${rows.join(",")}];`);
  }
}
