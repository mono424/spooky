import {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from '@spooky/query-builder';
import { createEffect, createSignal, onCleanup, useContext } from 'solid-js';
import { SyncedDb } from '..';
import { SpookyQueryResultPromise } from '@spooky/core';
import { SpookyContext } from './context';

type QueryArg<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
> =
  | FinalQuery<S, TableName, T, RelatedFields, IsOne, SpookyQueryResultPromise>
  | (() =>
      | FinalQuery<S, TableName, T, RelatedFields, IsOne, SpookyQueryResultPromise>
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
    const contextDb = useContext(SpookyContext);
    if (!contextDb) {
      throw new Error(
        'useQuery: No db argument provided and no SpookyContext found. ' +
        'Either pass a SyncedDb instance or wrap your app in <SpookyProvider>.'
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

  const spooky = db.getSpooky();

  const initQuery = async (
    query: FinalQuery<S, TableName, T, RelatedFields, IsOne, SpookyQueryResultPromise>
  ) => {
    const { hash } = await query.run();
    setError(undefined);

    const unsub = await spooky.subscribe(
      hash,
      (e) => {
        const data = (query.isOne ? e[0] : e) as TData;
        setData(() => data);
        setIsFetched(true);
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
      unsubscribe?.();
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
