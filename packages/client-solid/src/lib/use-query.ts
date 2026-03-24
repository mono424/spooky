import {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from '@spooky-sync/query-builder';
import { createEffect, createSignal, onCleanup, useContext } from 'solid-js';
import { SyncedDb } from '..';
import { Sp00kyQueryResultPromise } from '@spooky-sync/core';
import { Sp00kyContext } from './context';

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

type QueryOptions = { enabled?: () => boolean };

// Overload: context-based (no explicit db)
export function useQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>,
  options?: QueryOptions,
): { data: () => TData | undefined; error: () => Error | undefined; isLoading: () => boolean };

// Overload: explicit db (backward-compatible)
export function useQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  db: SyncedDb<S>,
  finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>,
  options?: QueryOptions,
): { data: () => TData | undefined; error: () => Error | undefined; isLoading: () => boolean };

// Implementation
export function useQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends {
    columns: Record<string, ColumnSchema>;
  },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null,
>(
  dbOrQuery:
    | SyncedDb<S>
    | QueryArg<S, TableName, T, RelatedFields, IsOne>,
  queryOrOptions?:
    | QueryArg<S, TableName, T, RelatedFields, IsOne>
    | QueryOptions,
  maybeOptions?: QueryOptions,
) {
  let db: SyncedDb<S>;
  let finalQuery: QueryArg<S, TableName, T, RelatedFields, IsOne>;
  let options: QueryOptions | undefined;

  if (dbOrQuery instanceof SyncedDb) {
    // Explicit db overload: useQuery(db, query, options?)
    db = dbOrQuery;
    finalQuery = queryOrOptions as QueryArg<S, TableName, T, RelatedFields, IsOne>;
    options = maybeOptions;
  } else {
    // Context-based overload: useQuery(query, options?)
    const contextDb = useContext(Sp00kyContext);
    if (!contextDb) {
      throw new Error(
        'useQuery: No db argument provided and no Sp00kyContext found. ' +
        'Either pass a SyncedDb instance or wrap your app in <Sp00kyProvider>.'
      );
    }
    db = contextDb as SyncedDb<S>;
    finalQuery = dbOrQuery;
    options = queryOrOptions as QueryOptions | undefined;
  }

  const [data, setData] = createSignal<TData | undefined>(undefined);
  const [error, setError] = createSignal<Error | undefined>(undefined);
  const [isFetched, setIsFetched] = createSignal(false);
  const [unsubscribe, setUnsubscribe] = createSignal<(() => void) | undefined>(undefined);
  let prevQueryString: string | undefined;

  const sp00ky = db.getSp00ky();

  const initQuery = async (
    query: FinalQuery<S, TableName, T, RelatedFields, IsOne, Sp00kyQueryResultPromise>
  ) => {
    const { hash } = await query.run();
    setError(undefined);

    let isFirstCall = true;
    const unsub = await sp00ky.subscribe(
      hash,
      (e) => {
        const data = (query.isOne ? e[0] : e) as TData;
        setData(() => data);
        // The first (immediate) callback with no data likely means the local DB
        // hasn't synced yet — don't mark as fetched so UI shows loading state
        const hasData = query.isOne ? data != null : (e as any[]).length > 0;
        if (!isFirstCall || hasData) {
          setIsFetched(true);
        }
        isFirstCall = false;
      },
      { immediate: true }
    );

    setUnsubscribe(() => unsub);
  };

  createEffect(() => {
    const enabled = options?.enabled?.() ?? true;

    // If disabled, clear error and don't run query
    if (!enabled) {
      setError(undefined);
      return;
    }

    // Init Query
    const query = typeof finalQuery === 'function' ? finalQuery() : finalQuery;
    if (!query) {
      return;
    }

    // Prevent re-running if query hasn't changed
    const queryString = JSON.stringify(query);
    if (queryString === prevQueryString) {
      return;
    }
    prevQueryString = queryString;

    // Reset fetched state when query changes
    setIsFetched(false);
    initQuery(query);

    // Cleanup
    onCleanup(() => {
      unsubscribe()?.();
    });
  });

  const isLoading = () => {
    return !isFetched() && error() === undefined;
  };

  return {
    data,
    error,
    isLoading,
  };
}
