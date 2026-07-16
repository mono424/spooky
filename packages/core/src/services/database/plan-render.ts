import type { QueryPlan, WhereNode, WhereComparison } from '@spooky-sync/query-builder';
import type { OrderBy, RelationFetch } from './cache-engine';

/**
 * Render helpers that turn an engine-neutral {@link QueryPlan} into a concrete
 * dialect. Two consumers: `SurrealCacheEngine` (SurrealQL) and, indirectly, the
 * SQLite worker (which uses the SQL variants). Relations are NOT rendered here —
 * they are resolved by `resolveRelations` via the {@link RelationFetch}
 * primitive, so both engines share the exact same decomposition.
 */

export interface RenderedQuery {
  sql: string;
  vars: Record<string, unknown>;
}

interface RenderCtx {
  vars: Record<string, unknown>;
  n: number;
}

function bind(ctx: RenderCtx, value: unknown): string {
  const name = `__p${ctx.n++}`;
  ctx.vars[name] = value;
  return `$${name}`;
}

function renderComparisonSurql(c: WhereComparison, ctx: RenderCtx): string {
  const right = c.paramRef ? `$${c.paramRef}` : bind(ctx, c.value);
  return c.swap ? `${right} ${c.op} ${c.field}` : `${c.field} ${c.op} ${right}`;
}

/** Render a WHERE conjunction (AND of comparisons / OR-groups) to SurrealQL. */
export function renderWhereSurql(nodes: WhereNode[], ctx: RenderCtx): string {
  return nodes
    .map((node) => {
      if ('or' in node) {
        return `(${node.or.map((c) => renderComparisonSurql(c, ctx)).join(' OR ')})`;
      }
      return renderComparisonSurql(node, ctx);
    })
    .join(' AND ');
}

function renderOrderBy(orderBy: OrderBy): string {
  return ` ORDER BY ${orderBy.map(([f, d]) => `${f} ${d}`).join(', ')}`;
}

/**
 * Render the BASE of a SELECT (no relations) to SurrealQL. `params` supplies
 * pre-existing bound params (e.g. `$__win` for windowing, or `paramRef` values)
 * and is merged into the returned vars.
 */
export function renderBaseSelectSurql(
  plan: QueryPlan,
  params: Record<string, unknown> = {}
): RenderedQuery {
  const ctx: RenderCtx = { vars: { ...params }, n: 0 };
  const projection = plan.select && plan.select.length > 0 ? plan.select.join(', ') : '*';
  let sql = `SELECT ${projection} FROM ${plan.table}`;
  if (plan.where && plan.where.length > 0) {
    sql += ` WHERE ${renderWhereSurql(plan.where, ctx)}`;
  }
  if (plan.orderBy && plan.orderBy.length > 0) sql += renderOrderBy(plan.orderBy);
  if (plan.limit !== undefined) sql += ` LIMIT ${plan.limit}`;
  if (plan.offset !== undefined) sql += ` START ${plan.offset}`;
  return { sql: `${sql};`, vars: ctx.vars };
}

/**
 * Render a batched relation fetch to SurrealQL:
 * `SELECT <select> FROM <table> WHERE <matchField> IN $__keys [AND <where>] [ORDER BY]`.
 * The resolver re-applies per-parent ORDER/LIMIT after grouping, so LIMIT is
 * intentionally omitted here.
 */
export function renderRelationFetchSurql(req: RelationFetch): RenderedQuery {
  const ctx: RenderCtx = { vars: { __keys: req.keys }, n: 0 };
  const projection =
    req.select && req.select.length > 0 ? ['id', ...req.select].join(', ') : '*';
  // The correlation keys arrive as record-id STRINGS (`"user:abc"`), but the
  // matched column (`id`, or a `record<…>` foreign key) is a RecordId. In
  // SurrealDB `id IN ["user:abc"]` never matches (string ≠ record), so every
  // `.related()` field would resolve empty. Coerce each key to a record id with
  // `type::record(<string> …)` (idempotent if a key is already a RecordId).
  let sql = `SELECT ${projection} FROM ${req.table} WHERE ${req.matchField} IN $__keys.map(|$__k| type::record(<string> $__k))`;
  if (req.where && req.where.length > 0) {
    sql += ` AND ${renderWhereSurql(req.where, ctx)}`;
  }
  if (req.orderBy && req.orderBy.length > 0) sql += renderOrderBy(req.orderBy);
  return { sql: `${sql};`, vars: ctx.vars };
}
