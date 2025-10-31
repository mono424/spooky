import { onCleanup, createEffect, Accessor, createSignal } from "solid-js";
import type { FinalQuery, SchemaStructure } from "@spooky/query-builder";
import { SyncedDbContext } from "..";

// Implementation
export function useQuery<T>(queryResult: Accessor<FinalQuery<T>>) {
  // Create internal signal to store data
  const [data, setData] = createSignal<T | null>(null);

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

  return data as any;
}
