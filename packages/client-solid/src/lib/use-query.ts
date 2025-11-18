import {
  ColumnSchema,
  FinalQuery,
  SchemaStructure,
  TableNames,
  QueryResult,
} from "@spooky/query-builder";
import { createEffect, createSignal, onCleanup } from "solid-js";
import { SyncedDb } from "..";

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
  db: SyncedDb<S>,
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
    query.run();
    console.log("[useQuery] init");
    const spooky = db.getSpooky();
    const subscriptionId = spooky.subscribeToQuery(
      query.hash,
      (e) => {
        const data = (query.isOne ? e[0] : e) as TData;
        console.log("[useQuery] Data updated", query.hash, data);
        setData(() => data);
      },
      { immediately: true }
    );

    const cleanup = () => {
      spooky.unsubscribeFromQuery(query.hash, subscriptionId);
    };

    onCleanup(() => {
      cleanup?.();
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
