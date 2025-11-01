import { onCleanup, createEffect, Accessor, createSignal } from "solid-js";
import type {
  ColumnSchema,
  FinalQuery,
  TableModel,
} from "@spooky/query-builder";

// Implementation
export function useQuery<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
>(queryResult: Accessor<FinalQuery<T, IsOne>>): [Accessor<TableModel<T>[]>] {
  // Create internal signal to store data
  const [data, setData] = createSignal<TableModel<T>[]>([]);

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

  return [data];
}

// Implementation
export function useQueryOne<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
>(queryResult: Accessor<FinalQuery<T, IsOne>>): [Accessor<TableModel<T>>] {
  const [dataArr] = useQuery(queryResult);

  const data = () => dataArr()[0];

  return [data];
}
