import { randomUUID } from "node:crypto";
import type { ViewRegisterRequest } from "../drivers/scheduler-http.js";

export interface GeneratedQuery extends ViewRegisterRequest {}

/**
 * Generates `count` distinct view-registration requests against the
 * test schema (user/thread/comment). Each query is a SELECT from one
 * of those tables filtered/parameterised by index.
 *
 * `clientId` is shared across all queries (a single benchmark client),
 * but `id` is unique per query so each is a separate registered view.
 */
export function generateQueries(count: number, clientId: string = randomUUID()): GeneratedQuery[] {
  // SSP requires both `ttl` (duration string) and `lastActiveAt` (datetime
  // string) on every registration, packages/ssp/src/service.rs:41-52.
  const ttl = "1h";
  const lastActiveAt = new Date().toISOString();
  const out: GeneratedQuery[] = [];
  for (let i = 0; i < count; i++) {
    const variant = i % 4;
    const id = `bench:q:${clientId}:${i}`;
    const ix = i;
    let surql: string;
    switch (variant) {
      case 0:
        surql = `SELECT * FROM user WHERE id = user:${ix} LIMIT 1`;
        break;
      case 1:
        surql = `SELECT * FROM thread WHERE author = user:${ix} ORDER BY created_at DESC LIMIT 50`;
        break;
      case 2:
        surql = `SELECT * FROM comment WHERE thread = thread:${ix} ORDER BY created_at DESC LIMIT 100`;
        break;
      default:
        surql = `SELECT id, title, author FROM thread WHERE title = "t${ix}" LIMIT 10`;
    }
    out.push({ id, surql, clientId, ttl, lastActiveAt });
  }
  return out;
}
