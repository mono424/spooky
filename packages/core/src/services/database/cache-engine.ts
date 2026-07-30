import type { QueryPlan, RelationPlan, WhereNode } from '@spooky-sync/query-builder';
import type { SealedQuery } from '../../utils/surql';
import type { DatabaseEventSystem } from './events/index';
import type { Sp00kyConfig, StorageHealth } from '../../types';

/**
 * A materialized row. Keys are field names; values are already decoded to the
 * client's runtime shapes (RecordId stays a RecordId, bytes a Uint8Array, …) so
 * every backend hands `DataModule` the same shape SurrealDB does today.
 */
export type Row = Record<string, unknown>;

/** A record identifier — a `RecordId` or its stable string form (`table:id`). */
export type Id = unknown;

/** How an order clause is expressed everywhere in the engine layer. */
export type OrderBy = [field: string, direction: 'asc' | 'desc'][];

/**
 * Batched relation fetch: "give me every row of `table` whose `matchField` is
 * one of `keys`, filtered by `where`, ordered by `orderBy`". This is the single
 * primitive relation decomposition (§3) leans on — implemented as
 * `SELECT … WHERE <matchField> IN (…)` on SQLite, `SELECT … FROM $keys` /
 * `WHERE <matchField> IN $keys` on SurrealDB, or an index scan elsewhere. Order
 * here is a hint; the resolver re-applies order+limit PER PARENT after grouping.
 */
export interface RelationFetch {
  table: string;
  matchField: string;
  keys: Id[];
  where?: WhereNode[];
  orderBy?: OrderBy;
  select?: string[];
}

/**
 * The read side of an engine, minus relation resolution — the surface a
 * {@link RelationResolver} needs. Kept separate so the resolver can be unit
 * tested against an in-memory fake without a full engine.
 */
export interface RowFetcher {
  /** Batched fan-out fetch. See {@link RelationFetch}. */
  fetchRelation(req: RelationFetch): Promise<Row[]>;
}

/** A transaction handle — the same verbs as the engine, but atomic. */
export interface EngineTx {
  upsert(table: string, id: Id, data: Row, mode: 'replace' | 'merge'): Promise<void>;
  patch(table: string, id: Id, patches: unknown[]): Promise<void>;
  delete(table: string, id: Id): Promise<void>;
}

/**
 * A pluggable local cache backend. SurrealDB (the default) and SQLite both
 * implement this; the rest of the client talks verbs, never SurrealQL.
 *
 * Reactivity is NOT part of this contract: the local cache is passive. The SSP
 * (remote) drives change; `DataModule` writes rows here and re-reads them. The
 * `epoch` field preserves the existing bucket-switch fencing (see
 * `LocalDatabaseService.epoch`): an async chain captures it at start and its
 * write is dropped if the epoch moved (a bucket switch) in between.
 */
export interface LocalCacheEngine extends RowFetcher {
  /** Monotonic store generation; bumped on every bucket switch. */
  readonly epoch: number;

  connect(bucketId: string): Promise<void>;
  switchBucket(bucketId: string): Promise<void>;
  close(): Promise<void>;

  /** Run `fn` inside a single atomic transaction. */
  transaction<T>(fn: (tx: EngineTx) => Promise<T>): Promise<T>;

  /**
   * Materialize a query, including its `.related()` tree (via §3
   * decomposition). Params bind `where` `paramRef`s and any windowing id-set.
   */
  select(plan: QueryPlan, params?: Record<string, unknown>): Promise<Row[]>;

  /** Fetch rows by primary id, preserving `ids` order; missing ids are skipped. */
  selectByIds(table: string, ids: Id[], opts?: { select?: string[]; orderBy?: OrderBy }): Promise<Row[]>;

  /** Single-record read by primary id, or `null`. */
  getById(table: string, id: Id): Promise<Row | null>;

  upsert(table: string, id: Id, data: Row, mode: 'replace' | 'merge'): Promise<void>;
  patch(table: string, id: Id, patches: unknown[]): Promise<void>;
  delete(table: string, id: Id): Promise<void>;
}

/**
 * The full surface the client's `this.local` field depends on: the
 * engine-neutral {@link LocalCacheEngine} verbs PLUS the legacy
 * SurrealQL/lifecycle methods the not-yet-migrated call sites still use.
 * `SurrealCacheEngine` (subclass of `LocalDatabaseService`) and
 * `SqliteCacheEngine` (via a SurrealQL-vocabulary shim) both satisfy this, so
 * either can back `this.local`.
 *
 * `getClient()` returns the underlying SurrealDB `Surreal` handle where one
 * exists (SurrealDB backend); backends without one (SQLite) throw — it is only
 * used by advanced/DevTools paths, never on the hot path.
 */
export interface LocalStore extends LocalCacheEngine {
  /**
   * Whether this engine needs SurrealQL schema provisioning (`DEFINE TABLE`,
   * `DEFINE FIELD`, …) run against it at init / bucket switch. SurrealDB → true;
   * schemaless engines (SQLite creates tables lazily) → false, so the client
   * skips the `LocalMigrator` entirely for them.
   */
  readonly usesSurqlSchema: boolean;
  query<T extends unknown[]>(
    query: string,
    vars?: Record<string, unknown>,
    opts?: { epoch?: number }
  ): Promise<T>;
  execute<T>(query: SealedQuery<T>, vars?: Record<string, unknown>, opts?: { epoch?: number }): Promise<T>;
  queryUngated<T extends unknown[]>(query: string, vars?: Record<string, unknown>): Promise<T>;
  switchStore(bucketId: string): Promise<void>;
  beginSwitch(): () => void;
  getEvents(): DatabaseEventSystem;
  getClient(): unknown;
  getConfig(): Sp00kyConfig<any>['database'];
  readonly currentBucketId: string;
  /**
   * Durability of this engine's local store. OPTIONAL: engines that don't
   * report it (SurrealDB, custom engines) are treated as `'unknown'` by the
   * client facade, so adding this needs no change on their side.
   */
  readonly storageHealth?: StorageHealth;
  /** Fires immediately with the current snapshot, then on every change.
   *  Returns an unsubscribe function. */
  subscribeToStorageHealth?(cb: (health: StorageHealth) => void): () => void;
}

/** Selected local cache backend. Mirrors the `persistenceClient` config pattern. */
export type LocalEngineChoice = 'surrealdb' | 'sqlite' | LocalStore;

/** Thrown when relation decomposition nests past {@link MAX_RELATION_DEPTH} —
 *  a guard against a cyclic schema producing unbounded fan-out. */
export class RelationCycleError extends Error {
  constructor(path: string[]) {
    super(`Relation nesting exceeded safe depth; possible cyclic schema: ${path.join(' -> ')}`);
    this.name = 'RelationCycleError';
  }
}

/** Defensive ceiling on relation nesting depth. A finite plan tree never hits
 *  this in practice; it exists so a malformed/cyclic plan fails loudly instead
 *  of running away. */
export const MAX_RELATION_DEPTH = 12;

export type { QueryPlan, RelationPlan, WhereNode };
