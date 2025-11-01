import { onCleanup, createEffect, Accessor, createSignal } from "solid-js";
import type {
  ColumnSchema,
  FinalQuery,
  TableModel,
} from "@spooky/query-builder";

// Helper type to extract the table type from FinalQuery
type ExtractTableType<T> = T extends FinalQuery<infer Table, any>
  ? Table
  : never;
type ExtractIsOne<T> = T extends FinalQuery<any, infer IsOne> ? IsOne : never;

// Conditional return type based on IsOne
type UseQueryReturn<T extends FinalQuery<any, any>> =
  ExtractIsOne<T> extends true
    ? Accessor<TableModel<ExtractTableType<T>> | undefined>
    : Accessor<TableModel<ExtractTableType<T>>[]>;

// Single signature with conditional return type
export function useQuery<T extends FinalQuery<any, any>>(
  queryResult: Accessor<T>
): [UseQueryReturn<T>] {
  type TableType = ExtractTableType<T>;

  // Create internal signal to store data
  const [data, setData] = createSignal<TableModel<TableType>[]>([]);

  // Track the previous query to detect changes
  let previousQueryHash: number | null = null;

  // Track the current query and cleanup
  createEffect(() => {
    const query = queryResult();
    if (query.hash === previousQueryHash) return;
    previousQueryHash = query.hash;

    const { data, subscribe } = query.select();
    setData(() => data);

    const unsubscribe = subscribe(setData);
    onCleanup(() => unsubscribe());
  });

  // Check if query is a "one" query once
  const isOneQuery = queryResult().isOne;

  // Return either single item or array based on isOne flag
  if (isOneQuery) {
    return [(() => data()[0]) as UseQueryReturn<T>];
  }

  return [data as UseQueryReturn<T>];
}
