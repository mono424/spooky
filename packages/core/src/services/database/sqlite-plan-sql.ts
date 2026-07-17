import type { WhereComparison, WhereNode } from '@spooky-sync/query-builder';
import { stableKey } from './relation-resolver';
import type { OrderBy, Row } from './cache-engine';

/**
 * SQL rendering + value (de)serialization for the SQLite cache backend.
 * Extracted from `sqlite-cache-engine.ts` so BOTH sides of the worker boundary
 * can use it: the engine (main thread) for the legacy/shim paths, and
 * `sqlite-worker.ts` for worker-side plan execution (`select`). Pure module —
 * no DOM, no worker, no engine imports — mirroring `plan-render.ts`.
 */

export function renderOrderSql(orderBy: OrderBy): string {
  return ` ORDER BY ${orderBy
    .map(([f, d]) => `json_extract(data, '$.${f}') ${d === 'desc' ? 'DESC' : 'ASC'}`)
    .join(', ')}`;
}

export function comparisonSql(
  c: WhereComparison,
  bind: unknown[],
  params: Record<string, unknown>
): string {
  const lhs = c.field === 'id' ? 'id' : `json_extract(data, '$.${c.field}')`;
  // Prefer the query's own param so a filter materializes from `params` (the
  // query's identity), not a baked literal. A pure `$`-ref has no `value`; a
  // slave-mode node keeps `value` as a fallback for a param absent from params.
  const useParam =
    c.paramRef !== undefined &&
    (c.value === undefined || Object.prototype.hasOwnProperty.call(params, c.paramRef));
  const value = useParam ? params[c.paramRef!] : c.value;
  bind.push(scalar(value));
  const op = c.op === '!=' ? '!=' : c.op;
  return c.swap ? `? ${op} ${lhs}` : `${lhs} ${op} ?`;
}

export function renderWhereSql(
  nodes: WhereNode[],
  bind: unknown[],
  params: Record<string, unknown>
): string {
  return nodes
    .map((node) => {
      if ('or' in node) {
        return `(${node.or.map((c) => comparisonSql(c, bind, params)).join(' OR ')})`;
      }
      return comparisonSql(node, bind, params);
    })
    .join(' AND ');
}

// ==================== value (de)serialization ====================

/** A comparable scalar for SQL binding: record links → `table:id`, everything
 *  else passed through (numbers/strings/bools). */
export function scalar(value: unknown): unknown {
  if (value == null) return null;
  if (typeof value === 'object') return stableKey(value);
  return value;
}

export function serializeRow(row: Row): string {
  return JSON.stringify(row, (_k, v) => {
    if (v instanceof Uint8Array) return { __u8: toBase64(v) };
    if (v && typeof v === 'object') {
      const rid = v as { tb?: unknown; id?: unknown };
      if (rid.tb !== undefined && rid.id !== undefined) return stableKey(v);
    }
    return v;
  });
}

export function reviveRow(json: string): Row {
  // Fast path: the per-key reviver is only needed to rebuild `Uint8Array`s from
  // `{__u8}` tags. Most rows (e.g. game bodies) have none — a plain parse avoids
  // invoking a JS callback for every key of every row on the read hot path.
  if (json.indexOf('"__u8"') === -1) return JSON.parse(json);
  return JSON.parse(json, (_k, v) => {
    if (v && typeof v === 'object' && typeof (v as any).__u8 === 'string') {
      return fromBase64((v as any).__u8);
    }
    return v;
  });
}

export function project(row: Row, fields: string[]): Row {
  const out: Row = {};
  for (const f of ['id', ...fields]) if (f in row) out[f] = row[f];
  return out;
}

export function toBase64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return typeof btoa !== 'undefined' ? btoa(bin) : Buffer.from(bytes).toString('base64');
}

export function fromBase64(b64: string): Uint8Array {
  if (typeof atob !== 'undefined') {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  return new Uint8Array(Buffer.from(b64, 'base64'));
}
