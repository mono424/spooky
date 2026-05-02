import type { ViewRegisterRequest } from "../drivers/scheduler-http.js";

/**
 * Domain 1.3 reframed as SurQL subquery depth. sp00ky doesn't support
 * registering a view that reads from another registered view (chained
 * views), but `Projection::Subquery` (`packages/ssp/src/operator/plan.rs`)
 * does support inlined subqueries, which exercise the same "compounding
 * propagation work" idea.
 *
 * Depth 1: flat select on `thread`.
 * Depth 2: thread with an inlined subquery over `comment`.
 * Depth 3: thread → comment subquery → user subquery.
 */

export function subqueryView(depth: 1 | 2 | 3, id: string, clientId: string): ViewRegisterRequest {
  // All depths root at `comment` so the comment-create workload exercises
  // every depth. Higher depths inline progressively more side queries.
  let surql: string;
  switch (depth) {
    case 1:
      surql = `SELECT id, content FROM comment`;
      break;
    case 2:
      surql =
        `SELECT id, content, ` +
        `(SELECT id, username FROM user WHERE id = $parent.author) AS author ` +
        `FROM comment`;
      break;
    case 3:
      surql =
        `SELECT id, content, ` +
        `(SELECT id, username FROM user WHERE id = $parent.author) AS author, ` +
        `(SELECT id, title FROM thread WHERE id = $parent.thread) AS thread_ref ` +
        `FROM comment`;
      break;
  }
  return {
    id,
    surql,
    clientId,
    ttl: "1h",
    lastActiveAt: new Date().toISOString(),
  };
}
