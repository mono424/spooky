import {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from '@spooky/query-builder';
import { createEffect, createSignal, onCleanup } from 'solid-js';
import { SyncedDb } from '..';
import { SpookyQueryResultPromise } from '@spooky/core';

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
  db: SyncedDb<S>,
  finalQuery:
    | FinalQuery<S, TableName, T, RelatedFields, IsOne, SpookyQueryResultPromise>
    | (() =>
        | FinalQuery<S, TableName, T, RelatedFields, IsOne, SpookyQueryResultPromise>
        | null
        | undefined),
  options?: { enabled?: () => boolean }
) {
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
