import type {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from '@spooky-sync/query-builder';
import {
  createMemo,
  createProjection,
  createSignal,
  onCleanup,
  type Accessor,
} from 'solid-js';
import { SyncedDb } from '..';
import type { Sp00kyQueryResultPromise } from '@spooky-sync/core';
import { useDb } from './context';
import { conflate } from './conflate';

type QueryArg<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
> =
  | FinalQuery<S, TableName, T, RelatedFields, IsOne, Sp00kyQueryResultPromise>
  | (() =>
      | FinalQuery<S, TableName, T, RelatedFields, IsOne, Sp00kyQueryResultPromise>
      | null
      | undefined);

export type QueryOptions = {
  enabled?: () => boolean;
  /**
   * Tear down the query (remote `_00_query` view + local WASM view) when this
   * hook is disposed and no other subscriber remains, instead of keeping it
   * resident for cheap re-subscription. Use for viewport-windowed lists that
   * mount/unmount a query per scroll window and want off-screen windows
   * cancelled. Trade-off: scrolling back to a torn-down window re-registers it.
   */
  deregisterOnCleanup?: boolean;
};

export type CreateQueryResult<TData> = {
  /**
   * Reactive result. Never suspends and never throws: born as an empty
   * committed value (`[]` / `null`) and reconciled in place (keyed by `id`) on
   * every live emission — unchanged rows keep identity, and coarse readers
   * (`<For>`) are notified on add/remove/reorder.
   */
  data: Accessor<TData>;
  /**
   * Suspending read of the same result for `<Loading>` users: throws Solid's
   * not-ready protocol until the query has delivered its first real result
   * (or errored, in which case it returns the empty value and `error()` is
   * set). Read this inside a `<Loading>` boundary.
   */
  ready: Accessor<TData>;
  error: Accessor<Error | undefined>;
  isLoading: Accessor<boolean>;
  isFetching: Accessor<boolean>;
  /**
   * True once the query has delivered a result AND no fetch cycle is in
   * flight (registration + initial sync included). While settled, results are
   * authoritative: a windowed query returning fewer rows than its LIMIT
   * really is the end of the list. Resets when the query identity changes.
   */
  isSettled: Accessor<boolean>;
};

// Overload: context-based (no explicit db)
export function createQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>,
  options?: QueryOptions
): CreateQueryResult<TData>;

// Overload: explicit db
export function createQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  db: SyncedDb<S>,
  finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>,
  options?: QueryOptions
): CreateQueryResult<TData>;

// Implementation
export function createQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends {
    columns: Record<string, ColumnSchema>;
  },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  dbOrQuery: SyncedDb<S> | QueryArg<S, TableName, T, RelatedFields, IsOne>,
  queryOrOptions?: QueryArg<S, TableName, T, RelatedFields, IsOne> | QueryOptions,
  maybeOptions?: QueryOptions
): CreateQueryResult<TData> {
  let db: SyncedDb<S>;
  let finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>;
  let options: QueryOptions | undefined;

  if (dbOrQuery instanceof SyncedDb) {
    db = dbOrQuery;
    finalQuery = queryOrOptions as QueryArg<S, TableName, T, RelatedFields, IsOne>;
    options = maybeOptions;
  } else {
    db = useDb<S>();
    finalQuery = dbOrQuery;
    options = queryOrOptions as QueryOptions | undefined;
  }

  const sp00ky = db.getSp00ky();

  // Status channel. Written from subscription callbacks and generator
  // continuations, which run outside any tracking scope — `ownedWrite` opts
  // these signals out of Solid 2's owned-scope write guard.
  const [error, setError] = createSignal<Error | undefined>(undefined, { ownedWrite: true });
  const [isFetched, setIsFetched] = createSignal(false, { ownedWrite: true });
  const [isFetching, setIsFetching] = createSignal(false, { ownedWrite: true });

  // The hash of the currently-installed subscription, for opt-in deregister on
  // dispose (see `deregisterOnCleanup`).
  let activeHash: string | undefined;

  // Results live in a projection: each yielded emission is reconciled in place
  // keyed by `id` (unchanged rows keep identity; coarse readers are notified
  // on add/remove/reorder — probed in rc-semantics.test.ts, replacing the
  // Solid 1 reconcile + version-signal hack). `seedLoadingValue` births the
  // store committed, so `data()` reads never suspend.
  //
  // The compute's tracked reads (enabled, query thunk) all happen before the
  // first await — Solid 2 only creates dependency edges for pre-await reads. A
  // dep change restarts the generator; the superseded one is ABANDONED by
  // Solid (no return()/finally — probed), so the onCleanup registered
  // synchronously below is what tears down its subscriptions. This also
  // replaces the Solid 1 hook's runId/prevQueryString supersede machinery:
  // Solid dedupes re-runs whose tracked reads are unchanged, and identical
  // query identity means an identical hash means the same compute inputs.
  const store = createProjection(
    async function* (): AsyncGenerator<{ value: TData }> {
      const enabled = options?.enabled?.() ?? true;
      const query = typeof finalQuery === 'function' ? finalQuery() : finalQuery;

      if (!enabled || !query) {
        setIsFetched(false);
        setError(undefined);
        return;
      }

      // A new identity starts clean: a previous identity's failure must not
      // keep this one out of its loading state.
      setIsFetched(false);
      setError(undefined);

      const iterators: AsyncIterator<any>[] = [];
      const cleanups: (() => void)[] = [];
      onCleanup(() => {
        for (const it of iterators) void it.return?.();
        for (const c of cleanups) c();
      });

      try {
        /**
         * Registration can fail — the canonical case is the SSP answering 503
         * NOT_READY while it bootstraps. Surface it as `error()` instead of
         * throwing into the graph: the sync scheduler retries the
         * registration underneath, so a transient failure still recovers, and
         * a spinner driven by `isLoading()` resolves via `error()`.
         */
        const { hash } = await query.run();
        activeHash = hash;

        // Mirror the query's fetch status so the UI can show a "loading more"
        // state while the sync engine pulls missing records in the background.
        cleanups.push(
          sp00ky.subscribeQueryStatus(hash, (status) => setIsFetching(status === 'fetching'), {
            immediate: true,
          })
        );

        const it = conflate<Record<string, any>[]>((cb) =>
          sp00ky.subscribe(hash, cb, { immediate: true })
        )[Symbol.asyncIterator]();
        iterators.push(it);

        let isFirstCall = true;
        while (true) {
          const r = await it.next();
          if (r.done) break;
          const e = r.value;
          const queryData = (query.isOne ? (e[0] ?? null) : e) as TData;
          // The first (immediate) callback with no data likely means the local
          // DB hasn't synced yet — don't mark as fetched so UI shows loading.
          const hasData = query.isOne
            ? queryData !== null && queryData !== undefined
            : e.length > 0;
          if (!isFirstCall || hasData) setIsFetched(true);
          isFirstCall = false;

          // Time the store commit (yield → resume) and report it as the
          // "frontend" phase for DevTools/MCP. Approximate: Solid reconciles
          // the yielded value before resuming the generator.
          const t0 = performance.now();
          yield { value: queryData };
          sp00ky.reportFrontendTiming(hash, performance.now() - t0);
        }
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
      }
    },
    // Wrapped in an object so `one()` queries (row object or null) and list
    // queries share one store shape; `key` reconciles `value`'s contents.
    { value: null as TData },
    { key: 'id', seedLoadingValue: true }
  );

  // Fallback empty value served before the first emission of a list query.
  const emptyList = [] as unknown as TData;

  const data: Accessor<TData> = () => {
    const v = store.value;
    if (v === null || v === undefined) {
      const query = typeof finalQuery === 'function' ? finalQuery() : finalQuery;
      if (query && !query.isOne) return emptyList;
    }
    return v as TData;
  };

  // Suspending read: pends until the first real result (or error) via an async
  // memo that resolves when isFetched/error flips. Reading it inside <Loading>
  // integrates with Solid 2's boundary protocol; `data` stays non-throwing.
  const readyGate = createMemo(async (): Promise<true> => {
    if (isFetched() || error()) return true;
    // Tracked reads above registered the deps; park until one flips.
    await new Promise<void>(() => {});
    return true;
  });
  const ready: Accessor<TData> = () => {
    readyGate();
    return data();
  };

  // Tear down the live subscription when the hook's owner is disposed. The
  // projection's own onCleanup (inside the compute) already unsubscribes; this
  // hook-scope cleanup only handles the opt-in query deregistration.
  onCleanup(() => {
    // Opt-in: cancel the query once this hook (its last subscriber) is gone.
    // The compute's cleanup removed this hook's callback, so deregisterQuery's
    // refcount guard sees the true remaining-subscriber count.
    if (options?.deregisterOnCleanup && activeHash) {
      sp00ky.deregisterQuery(activeHash);
    }
  });

  const isLoading = () => !isFetched() && error() === undefined;
  const isSettled = () => isFetched() && !isFetching();

  return {
    data,
    ready,
    error,
    isLoading,
    isFetching,
    isSettled,
  };
}

/** @deprecated Renamed `createQuery` in the Solid 2 binding. */
export const useQuery = createQuery;
