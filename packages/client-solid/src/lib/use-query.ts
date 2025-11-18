import {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from "@spooky/query-builder";
import { createEffect, createSignal, onCleanup } from "solid-js";

export function useQuery<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  T extends {
    columns: Record<string, ColumnSchema>;
  },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean,
  TData = QueryResult<S, TableName, RelatedFields, IsOne> | null
>(
  finalQuery:
    | FinalQuery<S, TableName, T, RelatedFields, IsOne>
    | (() =>
        | FinalQuery<S, TableName, T, RelatedFields, IsOne>
        | null
        | undefined),
  options?: { enabled?: () => boolean }
) {
  const [data, setData] = createSignal<TData | undefined>(undefined);
  const [error, setError] = createSignal<Error | undefined>(undefined);

  createEffect(() => {
    const enabled = options?.enabled?.() ?? true;

    // If disabled, clear error and don't run query
    if (!enabled) {
      setError(undefined);
      return;
    }

    const query = typeof finalQuery === "function" ? finalQuery() : finalQuery;
    if (!query) {
      return;
    }

    setError(undefined);

    let unsubscribe: (() => void) | undefined;
    let cancelled = false;

    try {
      const result = query.select();
      setData(() => result.data as TData);
      unsubscribe = result.subscribe((newData) => {
        if (!cancelled) setData(() => newData as TData);
      });
    } catch (err) {
      if (!cancelled) {
        setError(err instanceof Error ? err : new Error(String(err)));
      }
    }

    onCleanup(() => {
      cancelled = true;
      unsubscribe?.();
    });
  });

  const isLoading = () => {
    return data() === undefined && error() === undefined;
  };

  return {
    data,
    error,
    isLoading,
  };
}
