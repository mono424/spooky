import type { QueryPlan } from '@spooky-sync/query-builder';
import { resolveRelations, stableKey } from './relation-resolver';
import { renderOrderSql, renderWhereSql, reviveRow, project } from './sqlite-plan-sql';
import type { OrderBy, RelationFetch, Row, RowFetcher } from './cache-engine';

/**
 * Worker-side execution of a whole {@link QueryPlan} — table creation, base
 * select (either the `ids` window or where/order/limit), projection, and the
 * full `.related()` tree via the SHARED `resolveRelations` — against an
 * injected DB handle. Runs inside `sqlite-worker.ts` so the engine pays ONE
 * postMessage round-trip per select instead of one per table/relation level;
 * extracted into its own module so the logic is unit-testable off-worker
 * (parity with the engine's legacy multi-hop path).
 *
 * Semantics mirror `SqliteCacheEngine.selectLegacy` exactly: same SQL, same
 * ordering rules, same projection, same resolver.
 */

/** The slice of the worker's DB surface `executeSelect` needs. */
export interface SelectDb {
  /** Run a row-returning statement; rows come back as `{ data: <json> }`. */
  exec(sql: string, bind?: unknown[]): { data: string }[];
  /** Run a statement for effect only (CREATE TABLE). */
  run(sql: string, bind?: unknown[]): void;
  /** Tables already CREATEd on this handle (caller owns the lifecycle). */
  knownTables: Set<string>;
}

function ensureTable(db: SelectDb, table: string): void {
  if (db.knownTables.has(table)) return;
  db.run(`CREATE TABLE IF NOT EXISTS "${table}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)`);
  db.knownTables.add(table);
}

function execRows(db: SelectDb, sql: string, bind: unknown[]): Row[] {
  return db.exec(sql, bind).map((r) => reviveRow(r.data));
}

/** Mirrors the engine's `selectByIds`: fetch by primary id, preserving `ids`
 *  order unless an ORDER BY overrides it. */
function selectByIds(
  db: SelectDb,
  table: string,
  ids: unknown[],
  opts?: { select?: string[]; orderBy?: OrderBy }
): Row[] {
  if (ids.length === 0) return [];
  ensureTable(db, table);
  const keys = ids.map(stableKey);
  const placeholders = keys.map(() => '?').join(', ');
  let sql = `SELECT data FROM "${table}" WHERE id IN (${placeholders})`;
  if (opts?.orderBy && opts.orderBy.length > 0) sql += renderOrderSql(opts.orderBy);
  let rows = execRows(db, sql, keys);
  if (!opts?.orderBy || opts.orderBy.length === 0) {
    const pos = new Map(keys.map((k, i) => [k, i]));
    rows = rows.sort((a, b) => (pos.get(stableKey(a.id)) ?? 0) - (pos.get(stableKey(b.id)) ?? 0));
  }
  return opts?.select ? rows.map((r) => project(r, opts.select!)) : rows;
}

/** Mirrors the engine's `fetchRelation` SQL exactly. */
function fetchRelation(db: SelectDb, req: RelationFetch): Row[] {
  ensureTable(db, req.table);
  const keys = req.keys.map(stableKey);
  const placeholders = keys.map(() => '?').join(', ');
  const bind: unknown[] = [...keys];
  const lhs = req.matchField === 'id' ? 'id' : `json_extract(data, '$.${req.matchField}')`;
  let sql = `SELECT data FROM "${req.table}" WHERE ${lhs} IN (${placeholders})`;
  if (req.where && req.where.length > 0) {
    sql += ` AND ${renderWhereSql(req.where, bind, {})}`;
  }
  if (req.orderBy && req.orderBy.length > 0) sql += renderOrderSql(req.orderBy);
  const rows = execRows(db, sql, bind);
  return req.select ? rows.map((r) => project(r, req.select!)) : rows;
}

export async function executeSelect(
  plan: QueryPlan,
  params: Record<string, unknown>,
  db: SelectDb
): Promise<{ rows: Row[]; relationFetches: number }> {
  // Per-call fetch counter (the engine folds it into `__sqliteStats`).
  const counter = { n: 0 };
  const fetcher: RowFetcher = {
    fetchRelation: (req) => {
      counter.n++;
      return Promise.resolve(fetchRelation(db, req));
    },
  };
  // Window materialization: base rows are exactly `plan.ids`, ordered.
  if (plan.ids) {
    const rows = selectByIds(db, plan.table, plan.ids, {
      select: plan.select,
      orderBy: plan.orderBy,
    });
    await resolveRelations(rows, plan.relations, fetcher);
    return { rows, relationFetches: counter.n };
  }
  ensureTable(db, plan.table);
  const bind: unknown[] = [];
  let sql = `SELECT data FROM "${plan.table}"`;
  if (plan.where && plan.where.length > 0) {
    sql += ` WHERE ${renderWhereSql(plan.where, bind, params)}`;
  }
  if (plan.orderBy && plan.orderBy.length > 0) sql += renderOrderSql(plan.orderBy);
  // A query with no ORDER BY still has to render in SOME order, and "whatever
  // SQLite hands back" is insertion order — which disagrees with the order the
  // same query gets once it renders from server membership, and disagrees with
  // SurrealDB, whose natural order is by id. That mismatch is visible: the
  // first paint comes from this scan and the second from membership, so an
  // unordered list visibly reshuffled about a second after load. Ordering by
  // id here makes the two agree and makes the result stable across reloads.
  else sql += ` ORDER BY id`;
  if (plan.limit !== undefined) sql += ` LIMIT ${Number(plan.limit)}`;
  if (plan.offset !== undefined) sql += ` OFFSET ${Number(plan.offset)}`;
  const rows = execRows(db, sql, bind);
  // Optional projection trimming to match `SELECT <fields>`.
  const projected = plan.select ? rows.map((r) => project(r, plan.select!)) : rows;
  await resolveRelations(projected, plan.relations, fetcher);
  return { rows: projected, relationFetches: counter.n };
}
