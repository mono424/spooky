import { applyPatch, type Operation } from 'fast-json-patch';
import type { QueryPlan, RelationPlan, WhereNode } from '@spooky-sync/query-builder';
import {
  renderOrderSql,
  renderWhereSql,
  reviveRow,
  serializeRow,
  project,
} from './sqlite-plan-sql';
import type { Logger } from '../logger/index';
import type { Sp00kyConfig } from '../../types';
import type { SealedQuery } from '../../utils/surql';
import { resolveRelations, stableKey } from './relation-resolver';
import {
  createDatabaseEventSystem,
  DatabaseEventTypes,
  type DatabaseEventSystem,
} from './events/index';
import { StaleEpochError } from './local';
import { translateSurql, tableOf, setPath, getPath, type SqlOp } from './surql-translate';
import type { EngineTx, Id, LocalStore, OrderBy, RelationFetch, Row } from './cache-engine';

/**
 * The statement result a pure-write op contributes to a query's results array.
 * Single source of truth shared by the per-op path (`execOp`) and the batched
 * fast path in `query()`, so the two can never diverge: a caller that reads a
 * statement's output sees the same shape whether or not the transaction took the
 * batch fast path. In particular `create()` compiles to an all-upsert tx
 * (`createSet` + `createMutation`) and reads `resultIndex:0` for the new row and
 * its id — the fast path previously returned empty arrays there, so the row (and
 * its id) was lost and the reconcile crashed in `encodeRecordId`.
 *
 * An upsert echoes the written row (`{...data, id}`) with no read-back — the
 * full merged row is only materialized for a LET-wrapped upsert (see the 'let'
 * case). delete/deleteAll yield `[]`; noop yields `null`.
 */
/**
 * The `_00_*` internal tables the client relies on. The LocalMigrator DEFINEs
 * them, but every DEFINE lowers to a noop on the SQLite engine, so they must be
 * created physically at open (see `openInternal`) or a read-before-first-write
 * on a fresh bucket throws "no such table". Keep in sync with the systemSchema
 * block in `local-migrator.ts`.
 */
const SYSTEM_TABLES = [
  '_00_stream_processor_state',
  '_00_query',
  '_00_preload',
  '_00_schema',
  '_00_pending_mutations',
  // Server-written, synced-down meta tables (see meta_tables_client.surql).
  // DEFINE is a noop on this engine, so without seeding them here their synced
  // rows have no local table to land in: feature flags silently fall back to
  // defaults and app-release update notifications never show.
  '_00_user_feature',
  '_00_app_release',
] as const;

export function pureWriteOpResult(op: SqlOp): unknown {
  switch (op.kind) {
    case 'upsert':
      return { ...op.data, id: stableKey(op.id) };
    case 'noop':
      return null;
    default:
      return [];
  }
}

/**
 * Local cache backend on official SQLite-WASM in a dedicated Worker (see
 * `sqlite-worker.ts`), with OPFS SAHPool persistence. Storage model: one table
 * per schema table, `id TEXT PRIMARY KEY, data TEXT` where `data` is the row as
 * JSON. Filtering/ordering use `json_extract`. Relations are decomposed by the
 * shared {@link resolveRelations} — identical to the SurrealDB backend.
 *
 * Value normalization (JSON round-trip):
 * - `Uint8Array`/bytes → `{ "__u8": <base64> }` (CRDT snapshots survive).
 * - Record links / ids → their `table:id` string form (so `json_extract`
 *   comparisons and `IN` matching are consistent). NOTE: link fields therefore
 *   read back as strings, not `RecordId` instances — the one shape difference
 *   from the SurrealDB backend, to be closed with schema-driven revival + an
 *   oracle E2E in the browser.
 */
export class SqliteCacheEngine implements LocalStore {
  private worker: Worker | null = null;
  private seq = 0;
  private pending = new Map<number, { resolve: (v: any) => void; reject: (e: any) => void }>();
  private storeEpoch = 0;
  private knownTables = new Set<string>();
  private useOpfs: boolean;
  /** Whether `select` runs as one worker round-trip (plan executed in-worker).
   *  Flipped off at runtime if the worker script predates the `select` op
   *  (stale cached bundle) — degrade to the legacy multi-hop path, don't break. */
  private workerSelect: boolean;
  private events: DatabaseEventSystem = createDatabaseEventSystem();
  private bucketId = 'anon';
  /** Schemaless — tables are created lazily on first write; no migrator. */
  readonly usesSurqlSchema = false;

  constructor(
    private config: Sp00kyConfig<any>['database'],
    private logger: Logger,
    opts: { useOpfs?: boolean; workerSelect?: boolean } = {}
  ) {
    this.useOpfs = opts.useOpfs ?? true;
    this.workerSelect = opts.workerSelect ?? config.workerSelect ?? true;
  }

  get epoch(): number {
    return this.storeEpoch;
  }

  get currentBucketId(): string {
    return this.bucketId;
  }

  getConfig(): Sp00kyConfig<any>['database'] {
    return this.config;
  }

  getEvents(): DatabaseEventSystem {
    return this.events;
  }

  getClient(): unknown {
    throw new Error('SqliteCacheEngine has no SurrealDB client (getClient is unavailable).');
  }

  /** LocalStore alias; SQLite has no in-flight gate, so this maps to a rebuild. */
  switchStore(bucketId: string): Promise<void> {
    return this.switchBucket(bucketId);
  }

  /** SQLite has no switch gate window; the epoch bump alone fences stale writes. */
  beginSwitch(): () => void {
    return () => {};
  }

  // ---- worker plumbing -----------------------------------------------------

  private spawnWorker(): Worker {
    // Source references the `.ts` so the monorepo's src-bundling consumers
    // (e.g. the example app, which aliases `@spooky-sync/core` to `src`) resolve
    // it — Vite handles `.ts` workers. For the published package, the tsdown
    // build rewrites this to `./sqlite-worker.js` (the top-level emitted entry;
    // see tsdown.config.ts), which the flat `dist/index.js` resolves. The worker
    // (+ `@sqlite.org/sqlite-wasm`) still loads lazily — only when `localEngine:
    // 'sqlite'` is used.
    const worker = new Worker(new URL('./sqlite-worker.ts', import.meta.url), { type: 'module' });
    worker.onmessage = (ev: MessageEvent) => {
      const { id, ok, error, ...rest } = ev.data ?? {};
      const p = this.pending.get(id);
      if (!p) return;
      this.pending.delete(id);
      if (ok) p.resolve(rest);
      else p.reject(new Error(error));
    };
    // Surface a worker crash (wasm abort / OOM) instead of leaving every pending
    // call hung forever — reject them all with a clear error.
    const failAll = (msg: string) => {
      const err = new Error(`SQLite worker crashed: ${msg}`);
      this.logger.error(
        { err, Category: 'sp00ky-client::SqliteCacheEngine::worker' },
        'Worker error'
      );
      for (const [, p] of this.pending) p.reject(err);
      this.pending.clear();
    };
    worker.onerror = (e: ErrorEvent) => failAll(e.message || 'onerror');
    worker.onmessageerror = () => failAll('messageerror');
    return worker;
  }

  /** Serializes every worker op so reads/writes never overlap at the VFS layer
   *  (overlapping ops trip SQLITE_BUSY). Mirrors the SurrealDB engine's
   *  single-flight query queue. */
  private opQueue: Promise<unknown> = Promise.resolve();

  private call<T = any>(type: string, payload?: unknown): Promise<T> {
    const enqueuedAt = performance.now();
    const run = () => {
      // Time spent waiting behind other ops in the queue, not doing work.
      getStats().queueWaitMs += performance.now() - enqueuedAt;
      return this.rawCall<T>(type, payload);
    };
    const result = this.opQueue.then(run, run);
    // Keep the chain alive regardless of individual failures.
    this.opQueue = result.then(
      () => undefined,
      () => undefined
    );
    return result;
  }

  private rawCall<T = any>(type: string, payload?: unknown): Promise<T> {
    if (!this.worker) throw new Error('SqliteCacheEngine: not connected');
    const id = ++this.seq;
    // --- instrumentation: live, inspectable via `globalThis.__sqliteStats` ---
    const s = getStats();
    s.roundTrips++;
    s.byType[type] = (s.byType[type] ?? 0) + 1;
    if (type === 'batch' && Array.isArray(payload)) {
      s.batchStatements += payload.length;
      s.maxBatch = Math.max(s.maxBatch, payload.length);
    }
    s.inFlight++;
    s.maxInFlight = Math.max(s.maxInFlight, s.inFlight);
    const done = () => {
      s.inFlight--;
    };
    const sentAt = performance.now();
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (v: T) => {
          done();
          // Split the round-trip: `wt` is time inside the worker's handler,
          // the remainder is postMessage + scheduling overhead.
          const wt = (v as { wt?: unknown } | null)?.wt;
          if (typeof wt === 'number') {
            s.workerMs += wt;
            s.rpcOverheadMs += Math.max(0, performance.now() - sentAt - wt);
          }
          resolve(v);
        },
        reject: (e: unknown) => {
          done();
          reject(e);
        },
      });
      this.worker!.postMessage({ id, type, payload });
    });
  }

  /**
   * Spawn the worker and open `bucketId`'s DB. Uses {@link rawCall} (NOT
   * {@link call}) so it can run as the body of an already-queued opQueue entry
   * without re-queuing onto itself. Callers must run it through the opQueue.
   */
  private async openInternal(bucketId: string): Promise<void> {
    this.worker = this.spawnWorker();
    // Seed the `_00_*` system tables as part of `open` (worker-side, one round
    // trip). The LocalMigrator DEFINEs them, but `translateSurql` lowers every
    // DEFINE to a noop on this engine (SQLite has no DDL vocabulary), so
    // provisioning never actually creates them — they were only made lazily on
    // first WRITE. A fresh bucket (e.g. right after signup) that READS one first
    // (the sync layer selects `_00_query` before any row lands) hit
    // "no such table: _00_query" and the client wedged on "Loading database".
    // Creating them inside `open` guarantees any access order is safe without
    // adding ops to the engine's queue.
    const { persisted } = await this.rawCall<{ persisted: boolean }>('open', {
      dbName: bucketId,
      useOpfs: this.useOpfs,
      systemTables: SYSTEM_TABLES,
    });
    this.knownTables.clear();
    for (const t of SYSTEM_TABLES) this.knownTables.add(t);
    this.bucketId = bucketId;
    this.logger.info(
      { bucketId, persisted, Category: 'sp00ky-client::SqliteCacheEngine::connect' },
      persisted ? 'SQLite OPFS store opened' : 'SQLite in-memory store opened (no OPFS)'
    );
  }

  /** Enqueue `fn` as a single serialized opQueue entry (mirrors {@link call}'s
   *  chaining) so it can't interleave with reads/writes at the worker. */
  private enqueue<T>(fn: () => Promise<T>): Promise<T> {
    const result = this.opQueue.then(fn, fn);
    this.opQueue = result.then(
      () => undefined,
      () => undefined
    );
    return result;
  }

  async connect(bucketId: string): Promise<void> {
    // Serialize through the opQueue so an op racing boot can't dispatch to a
    // half-open worker.
    await this.enqueue(() => this.openInternal(bucketId));
  }

  async switchBucket(bucketId: string): Promise<void> {
    this.storeEpoch++;
    // Run close → terminate → reopen as ONE opQueue entry. Previously `close`
    // and the new `open` were separate entries, so a query's `exec` (e.g. the
    // query re-registration fired by an auth/bucket change) could slot in
    // between and dispatch to the just-closed worker → "sqlite: DB not open".
    // As a single entry, every other op runs fully before the close or after
    // the reopen — never against a closed DB.
    await this.enqueue(async () => {
      if (this.worker) {
        try {
          await this.rawCall('close');
        } catch {
          /* ignore */
        }
        this.worker.terminate();
        this.worker = null;
      }
      await this.openInternal(bucketId);
    });
  }

  async close(): Promise<void> {
    if (!this.worker) return;
    try {
      await this.call('close');
    } catch {
      /* ignore */
    }
    this.worker.terminate();
    this.worker = null;
  }

  private async ensureTable(table: string): Promise<void> {
    if (this.knownTables.has(table)) return;
    await this.call('run', {
      sql: `CREATE TABLE IF NOT EXISTS "${table}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)`,
    });
    this.knownTables.add(table);
  }

  private async execRows(sql: string, bind: unknown[]): Promise<Row[]> {
    const { rows } = await this.call<{ rows: { data: string }[] }>('exec', { sql, bind });
    const s = getStats();
    const t0 = performance.now();
    const out = (rows ?? []).map((r) => {
      s.bytesParsed += r.data.length;
      return reviveRow(r.data);
    });
    s.parseMs += performance.now() - t0;
    s.rowsParsed += out.length;
    return out;
  }

  // ---- reads ---------------------------------------------------------------

  async select(plan: QueryPlan, params: Record<string, unknown> = {}): Promise<Row[]> {
    if (this.workerSelect) {
      // ONE round-trip: the worker executes the whole plan (table creation,
      // base select, relation tree, JSON parse) and returns structured-clone
      // rows. The legacy path below pays a postMessage hop per table/relation
      // level plus main-thread parsing — the dominant first-load cost.
      //
      // Normalize before postMessage: class-instance VALUES (RecordId & co) →
      // their `stableKey` string. structuredClone strips a class instance to a
      // bare plain object — and surrealdb's RecordId keeps its data behind
      // getters (no own properties), so it clones to `{}`: the worker would
      // filter on garbage. Applies to params AND to values baked inside the
      // plan (where nodes, relation sub-wheres, window ids).
      // Param KEYS must pass through untouched — `comparisonSql` resolves
      // `paramRef` via hasOwnProperty, and a dropped key silently falls back to
      // the baked literal (the crossed-results class fixed in aa4af79b).
      const normPlan = normalizePlanForClone(plan);
      const normParams: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(params)) normParams[k] = toCloneSafe(v);
      try {
        const res = await this.call<{ rows: Row[]; relationFetches?: number }>('select', {
          plan: normPlan,
          params: normParams,
        });
        getStats().relationFetches += res.relationFetches ?? 0;
        return res.rows ?? [];
      } catch (err) {
        // Stale worker script without the 'select' op: fall back for good.
        if (err instanceof Error && err.message.includes('unknown message')) {
          this.workerSelect = false;
          this.logger.warn(
            { err, Category: 'sp00ky-client::SqliteCacheEngine::select' },
            'Worker lacks select op (stale script?) — falling back to multi-hop select'
          );
        } else {
          throw err;
        }
      }
    }
    return this.selectLegacy(plan, params);
  }

  /** Pre-worker-select path: one worker round-trip per table/relation level,
   *  rows parsed on the main thread. Kept as the `workerSelect:false` escape
   *  hatch and the stale-worker fallback. */
  private async selectLegacy(
    plan: QueryPlan,
    params: Record<string, unknown> = {}
  ): Promise<Row[]> {
    // Window materialization: base rows are exactly `plan.ids`, ordered.
    if (plan.ids) {
      const result = await this.selectByIds(plan.table, plan.ids, {
        select: plan.select,
        orderBy: plan.orderBy,
      });
      await resolveRelations(result, plan.relations, this);
      return result;
    }
    await this.ensureTable(plan.table);
    const bind: unknown[] = [];
    const proj = 'data';
    let sql = `SELECT ${proj} FROM "${plan.table}"`;
    if (plan.where && plan.where.length > 0) {
      sql += ` WHERE ${renderWhereSql(plan.where, bind, params)}`;
    }
    if (plan.orderBy && plan.orderBy.length > 0) sql += renderOrderSql(plan.orderBy);
    if (plan.limit !== undefined) sql += ` LIMIT ${Number(plan.limit)}`;
    if (plan.offset !== undefined) sql += ` OFFSET ${Number(plan.offset)}`;
    const rows = await this.execRows(sql, bind);
    // Optional projection trimming to match `SELECT <fields>`.
    const projected = plan.select ? rows.map((r) => project(r, plan.select!)) : rows;
    await resolveRelations(projected, plan.relations, this);
    return projected;
  }

  async fetchRelation(req: RelationFetch): Promise<Row[]> {
    getStats().relationFetches++;
    await this.ensureTable(req.table);
    const keys = req.keys.map(stableKey);
    const placeholders = keys.map(() => '?').join(', ');
    const bind: unknown[] = [...keys];
    const lhs = req.matchField === 'id' ? 'id' : `json_extract(data, '$.${req.matchField}')`;
    let sql = `SELECT data FROM "${req.table}" WHERE ${lhs} IN (${placeholders})`;
    if (req.where && req.where.length > 0) {
      sql += ` AND ${renderWhereSql(req.where, bind, {})}`;
    }
    if (req.orderBy && req.orderBy.length > 0) sql += renderOrderSql(req.orderBy);
    const rows = await this.execRows(sql, bind);
    return req.select ? rows.map((r) => project(r, req.select!)) : rows;
  }

  async selectByIds(
    table: string,
    ids: Id[],
    opts?: { select?: string[]; orderBy?: OrderBy }
  ): Promise<Row[]> {
    if (ids.length === 0) return [];
    await this.ensureTable(table);
    const keys = ids.map(stableKey);
    const placeholders = keys.map(() => '?').join(', ');
    let sql = `SELECT data FROM "${table}" WHERE id IN (${placeholders})`;
    if (opts?.orderBy && opts.orderBy.length > 0) sql += renderOrderSql(opts.orderBy);
    let rows = await this.execRows(sql, keys);
    if (!opts?.orderBy || opts.orderBy.length === 0) {
      const pos = new Map(keys.map((k, i) => [k, i]));
      rows = rows.sort((a, b) => (pos.get(stableKey(a.id)) ?? 0) - (pos.get(stableKey(b.id)) ?? 0));
    }
    return opts?.select ? rows.map((r) => project(r, opts.select!)) : rows;
  }

  async getById(table: string, id: Id): Promise<Row | null> {
    await this.ensureTable(table);
    const rows = await this.execRows(`SELECT data FROM "${table}" WHERE id = ?`, [stableKey(id)]);
    return rows[0] ?? null;
  }

  // ---- writes --------------------------------------------------------------

  async upsert(table: string, id: Id, data: Row, mode: 'replace' | 'merge'): Promise<void> {
    await this.ensureTable(table);
    const key = stableKey(id);
    if (mode === 'merge') {
      // Merge in-SQL via json_patch (RFC7396 = MERGE semantics): on insert store
      // the row, on conflict shallow-merge. Serialize once, reuse for VALUES and
      // the patch. No read-modify-write round-trip. (RFC7396: null deletes key.)
      const full = serializeRow({ ...data, id: key });
      await this.call('run', {
        sql: `INSERT INTO "${table}"(id, data) VALUES(?, ?) ON CONFLICT(id) DO UPDATE SET data = json_patch(data, ?)`,
        bind: [key, full, full],
      });
      return;
    }
    await this.call('run', {
      sql: `INSERT INTO "${table}"(id, data) VALUES(?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data`,
      bind: [key, serializeRow({ ...data, id: key })],
    });
  }

  async patch(table: string, id: Id, patches: unknown[]): Promise<void> {
    // RFC6902 (fast-json-patch) applied read-modify-write — SQLite's json_patch
    // is RFC7396 merge-patch and would misinterpret op arrays.
    const existing = (await this.getById(table, id)) ?? { id: stableKey(id) };
    const next = applyPatch(existing, patches as Operation[]).newDocument as Row;
    await this.upsert(table, id, next, 'replace');
  }

  async delete(table: string, id: Id): Promise<void> {
    await this.ensureTable(table);
    await this.call('run', { sql: `DELETE FROM "${table}" WHERE id = ?`, bind: [stableKey(id)] });
  }

  // ---- SurrealQL-vocabulary shim (LocalStore compatibility) ---------------

  /**
   * Execute a raw SurrealQL statement by translating the client's bounded
   * vocabulary to verbs (see `surql-translate.ts`). Returns results shaped like
   * SurrealDB's `.query()` (one element per statement; a tx prepends a `null`
   * begin-result so `surql.seal` extraction lines up). Epoch-fences writes.
   */
  async query<T extends unknown[]>(
    sql: string,
    vars: Record<string, unknown> = {},
    opts?: { epoch?: number }
  ): Promise<T> {
    if (opts?.epoch !== undefined && opts.epoch !== this.storeEpoch) throw new StaleEpochError();
    const start = performance.now();
    try {
      const { transaction, ops } = translateSurql(sql, vars);
      let shaped: unknown[];
      // FAST PATH: a pure-write transaction (bulk sync-down is one
      // `tx([upsertMerge…])`) compiles to a SINGLE worker `batch` message run in
      // one SQLite transaction — instead of 1-2 worker round-trips PER row. This
      // is the dominant sync-down cost; per-op execution here caused the churn
      // OOM. Mixed txs (LET/RETURN single-record mutations) keep the per-op path.
      if (
        transaction &&
        ops.every(
          (o) =>
            o.kind === 'upsert' ||
            o.kind === 'delete' ||
            o.kind === 'deleteAll' ||
            o.kind === 'noop'
        )
      ) {
        await this.runWriteBatch(ops);
        // Shape each statement's result the SAME as the per-op path (`execOp`),
        // so a caller reading a statement's output still works after taking the
        // batch fast path. A single `create()` compiles to an all-upsert tx
        // (createSet + createMutation) and reads `resultIndex:0` for the new
        // row + its id; returning empty arrays here dropped that row (id became
        // undefined → the reconcile crashed in `encodeRecordId`). The
        // single-batch write is kept — this only rebuilds the return value.
        shaped = [null, ...ops.map(pureWriteOpResult)];
      } else {
        const results: unknown[] = [];
        // Per-query scope holds `LET $var = (...)` bindings for later statements
        // (e.g. `RETURN { target: $updated }`).
        const scope: Record<string, unknown> = {};
        for (const op of ops) results.push(await this.execOp(op, scope, vars));
        shaped = transaction ? [null, ...results] : results;
      }
      this.events.emit(DatabaseEventTypes.LocalQuery, {
        query: sql,
        vars,
        duration: performance.now() - start,
        success: true,
        timestamp: Date.now(),
      });
      return shaped as unknown as T;
    } catch (err) {
      this.events.emit(DatabaseEventTypes.LocalQuery, {
        query: sql,
        vars,
        duration: performance.now() - start,
        success: false,
        error: err instanceof Error ? err.message : String(err),
        timestamp: Date.now(),
      });
      throw err;
    }
  }

  async execute<R>(
    query: SealedQuery<R>,
    vars?: Record<string, unknown>,
    opts?: { epoch?: number }
  ): Promise<R> {
    const raw = await this.query<unknown[]>(query.sql, vars, opts);
    return query.extract(raw);
  }

  queryUngated<T extends unknown[]>(sql: string, vars?: Record<string, unknown>): Promise<T> {
    return this.query<T>(sql, vars);
  }

  private async execOp(
    op: SqlOp,
    scope: Record<string, unknown>,
    vars: Record<string, unknown>
  ): Promise<unknown> {
    switch (op.kind) {
      case 'getById': {
        const row = await this.getById(tableOf(op.id), op.id);
        if (op.value) return row ? (row[op.value] ?? null) : null;
        return row ? (op.select ? project(row, op.select) : row) : null;
      }
      case 'selectByIds': {
        if (op.ids.length === 0) return [];
        let rows = await this.selectByIds(tableOf(op.ids[0]), op.ids, {
          select: op.select,
          orderBy: op.orderBy,
        });
        if (op.value) return rows.map((r) => r[op.value!]);
        return rows;
      }
      case 'selectTable': {
        const rows = await this.rawSelectTable(op.table, op.where, op.orderBy);
        if (op.value) return rows.map((r) => r[op.value!]);
        return op.select ? rows.map((r) => project(r, op.select!)) : rows;
      }
      case 'upsert':
        await this.upsert(tableOf(op.id), op.id, op.data, op.mode);
        // Cheap return — no read-back. The full merged row is only needed by a
        // LET-wrapped upsert, which reads it back in the 'let' case below. This
        // avoids an extra worker round-trip + full-row parse on EVERY sync-down
        // write (the hot path under rapid churn). Shared with the batch fast
        // path so the two never drift.
        return pureWriteOpResult(op);
      case 'updateSet': {
        const existing = (await this.getById(tableOf(op.id), op.id)) ?? { id: stableKey(op.id) };
        for (const { path, op: setOp, value } of op.sets) {
          if (setOp === '+=' || setOp === '-=') {
            const cur = Number(getPath(existing, path) ?? 0);
            const delta = Number(value ?? 0);
            setPath(existing, path, setOp === '+=' ? cur + delta : cur - delta);
          } else {
            setPath(existing, path, value);
          }
        }
        await this.upsert(tableOf(op.id), op.id, existing, 'replace');
        return op.returnNone ? null : existing;
      }
      case 'delete':
        await this.delete(tableOf(op.id), op.id);
        return pureWriteOpResult(op);
      case 'deleteAll':
        await this.ensureTable(op.table);
        await this.call('run', { sql: `DELETE FROM "${op.table}"` });
        return pureWriteOpResult(op);
      case 'let': {
        let result = await this.execOp(op.inner, scope, vars);
        // A LET-bound UPSERT must expose the FULL merged row (e.g.
        // `RETURN { target: $updated }`), so read it back here — only here,
        // not on every upsert.
        if (op.inner.kind === 'upsert') {
          result = (await this.getById(tableOf(op.inner.id), op.inner.id)) ?? result;
        }
        scope[op.var] = result;
        return result;
      }
      case 'return': {
        const obj: Row = {};
        for (const { key, var: v } of op.entries) {
          obj[key] = v in scope ? scope[v] : vars[v];
        }
        return obj;
      }
      case 'noop':
        return pureWriteOpResult(op);
    }
  }

  /**
   * Compile a pure-write op list to ONE worker `batch` (single SQLite
   * transaction). Ensures each touched table exists, then one statement per op.
   * Merges happen in-SQL via json_patch — no read-back round-trips.
   */
  private async runWriteBatch(ops: SqlOp[]): Promise<void> {
    const stmts: { sql: string; bind?: unknown[] }[] = [];
    const tables = new Set<string>();
    const ensure = (t: string) => {
      if (!tables.has(t)) {
        tables.add(t);
        this.knownTables.add(t);
        stmts.push({
          sql: `CREATE TABLE IF NOT EXISTS "${t}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)`,
        });
      }
    };
    for (const op of ops) {
      if (op.kind === 'upsert') {
        const t = tableOf(op.id);
        const key = stableKey(op.id);
        ensure(t);
        if (op.mode === 'merge') {
          // Serialize ONCE and reuse for both VALUES (fresh insert) and the
          // json_patch (merge). Patching with `id` is a harmless no-op set, so
          // the full row doubles as the delta — halves per-row stringify cost.
          const full = serializeRow({ ...op.data, id: key });
          stmts.push({
            sql: `INSERT INTO "${t}"(id, data) VALUES(?, ?) ON CONFLICT(id) DO UPDATE SET data = json_patch(data, ?)`,
            bind: [key, full, full],
          });
        } else {
          stmts.push({
            sql: `INSERT INTO "${t}"(id, data) VALUES(?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data`,
            bind: [key, serializeRow({ ...op.data, id: key })],
          });
        }
      } else if (op.kind === 'delete') {
        const t = tableOf(op.id);
        ensure(t);
        stmts.push({ sql: `DELETE FROM "${t}" WHERE id = ?`, bind: [stableKey(op.id)] });
      } else if (op.kind === 'deleteAll') {
        ensure(op.table);
        stmts.push({ sql: `DELETE FROM "${op.table}"` });
      }
    }
    if (stmts.length > 0) await this.call('batch', stmts);
  }

  private async rawSelectTable(
    table: string,
    where?: WhereNode[],
    orderBy?: OrderBy
  ): Promise<Row[]> {
    await this.ensureTable(table);
    const bind: unknown[] = [];
    let sql = `SELECT data FROM "${table}"`;
    if (where && where.length > 0) sql += ` WHERE ${renderWhereSql(where, bind, {})}`;
    if (orderBy && orderBy.length > 0) sql += renderOrderSql(orderBy);
    return this.execRows(sql, bind);
  }

  async transaction<T>(fn: (tx: EngineTx) => Promise<T>): Promise<T> {
    // Verbs run in order on the single worker message channel. `patch` needs a
    // read-modify-write round-trip, so a single BEGIN/COMMIT batch cannot wrap
    // the whole closure; sequential execution is sufficient for the current
    // single-record write sites.
    const tx: EngineTx = {
      upsert: (t, id, data, mode) => this.upsert(t, id, data, mode),
      patch: (t, id, p) => this.patch(t, id, p),
      delete: (t, id) => this.delete(t, id),
    };
    return fn(tx);
  }
}

// ==================== instrumentation ====================

interface SqliteStats {
  roundTrips: number;
  batchStatements: number;
  maxBatch: number;
  inFlight: number;
  maxInFlight: number;
  byType: Record<string, number>;
  /** Time ops spent waiting behind the opQueue before dispatch. */
  queueWaitMs: number;
  /** Time inside the worker's message handler (actual SQLite work). */
  workerMs: number;
  /** Round-trip time minus workerMs: postMessage + scheduling overhead. */
  rpcOverheadMs: number;
  /** Main-thread JSON parse/revive of returned rows. */
  parseMs: number;
  rowsParsed: number;
  bytesParsed: number;
  /** Relation-resolver fan-out fetches (one worker round-trip each). */
  relationFetches: number;
}

const EMPTY_STATS: SqliteStats = {
  roundTrips: 0,
  batchStatements: 0,
  maxBatch: 0,
  inFlight: 0,
  maxInFlight: 0,
  byType: {},
  queueWaitMs: 0,
  workerMs: 0,
  rpcOverheadMs: 0,
  parseMs: 0,
  rowsParsed: 0,
  bytesParsed: 0,
  relationFetches: 0,
};

/** Live stats, inspectable in the browser console via `__sqliteStats`. Counts
 *  worker round-trips (the sync-down cost driver), batch sizes, queue depth
 *  (`maxInFlight`), and the latency split of each round-trip (queue wait vs
 *  worker time vs RPC overhead vs main-thread row parsing) so first-load cost
 *  can be measured rather than guessed. */
function getStats(): SqliteStats {
  const g = globalThis as unknown as { __sqliteStats?: SqliteStats };
  if (!g.__sqliteStats) {
    g.__sqliteStats = { ...EMPTY_STATS, byType: {} };
  } else {
    // Backfill fields added since the object was created (HMR / older bundle).
    for (const [k, v] of Object.entries(EMPTY_STATS)) {
      if ((g.__sqliteStats as any)[k] === undefined) (g.__sqliteStats as any)[k] = v;
    }
  }
  return g.__sqliteStats;
}

// SQL rendering + row (de)serialization live in `sqlite-plan-sql.ts`, shared
// with the worker (worker-side plan execution renders the same SQL).

// ==================== structured-clone normalization ====================

/**
 * Make a bind/param value safe to cross the worker boundary. structuredClone
 * keeps plain data intact but strips a CLASS instance to a bare object — and
 * surrealdb's `RecordId` stores its fields behind getters (zero own
 * properties), so it clones to `{}`. Convert such instances to their
 * `stableKey` string (the exact value `scalar()` would bind on the main
 * thread), leave everything clone-representable untouched.
 */
function toCloneSafe(v: unknown): unknown {
  if (v === null || typeof v !== 'object') return v;
  if (Array.isArray(v) || v instanceof Uint8Array || v instanceof Date) return v;
  const proto = Object.getPrototypeOf(v);
  if (proto === Object.prototype || proto === null) return v;
  return stableKey(v);
}

function normalizeWhereForClone(nodes: WhereNode[] | undefined): WhereNode[] | undefined {
  if (!nodes) return nodes;
  return nodes.map((n) =>
    'or' in n
      ? { or: n.or.map((c) => ({ ...c, value: toCloneSafe(c.value) })) }
      : { ...n, value: toCloneSafe(n.value) }
  );
}

function normalizeRelationForClone(r: RelationPlan): RelationPlan {
  return {
    ...r,
    where: normalizeWhereForClone(r.where),
    relations: r.relations?.map(normalizeRelationForClone),
  };
}

/** Normalize every baked value a plan carries (where trees, window ids) for
 *  the postMessage to the worker's `select` op. */
function normalizePlanForClone(plan: QueryPlan): QueryPlan {
  return {
    ...plan,
    ids: plan.ids?.map(stableKey),
    where: normalizeWhereForClone(plan.where),
    relations: plan.relations?.map(normalizeRelationForClone),
  };
}
