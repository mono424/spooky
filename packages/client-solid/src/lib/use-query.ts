import { createSignal, onCleanup, createEffect, Accessor } from "solid-js";
import { ReactiveQueryResult } from "./table-queries";
import { GenericModel } from "./models";
import { snapshot, Snapshot, subscribe } from "valtio";

/**
 * A SolidJS hook that subscribes to a ReactiveQueryResult and returns a signal
 *
 * @param queryResult - The reactive query result to subscribe to
 * @returns A signal accessor that returns the current data array
 *
 * @example
 * ```tsx
 * const threadsQuery = db.query.thread
 *   .find({})
 *   .orderBy("created_at", "desc")
 *   .query();
 *
 * const threads = useQuery(threadsQuery);
 *
 * // Use in JSX
 * <For each={threads()}>
 *   {(thread) => <div>{thread.title}</div>}
 * </For>
 * ```
 */
export function useQuery<Model extends GenericModel>(
  queryResult: ReactiveQueryResult<Model>,
  setData: (data: readonly Snapshot<Model>[]) => void
): void {
  // const [data, setData] = createSignal<readonly Snapshot<Model>[]>(
  //   snapshot(queryResult.data),
  //   {
  //     equals: false, // Always trigger updates since we're subscribing to a proxy
  //   }
  // );

  // Subscribe to changes in the query result data
  const unsubscribe = subscribe(queryResult.data, () => {
    setData(snapshot(queryResult.data));
  });

  // Initial sync to ensure we have the latest data
  createEffect(() => {
    setData(snapshot(queryResult.data));
  });

  // Clean up subscription when component unmounts
  onCleanup(() => {
    unsubscribe();
    queryResult.kill();
  });

  // return [data];
}
