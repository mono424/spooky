import { createSignal, onCleanup, createEffect, Accessor } from "solid-js";
import { ReactiveQueryResult, ReactiveQueryResultOne } from "./table-queries";
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
export function useQuery<Model extends Record<string, any>>(
  queryResult: ReactiveQueryResult<Model>,
  setData: (data: readonly Snapshot<Model>[]) => void
): void;

export function useQuery<Model extends Record<string, any>>(
  queryResult: ReactiveQueryResultOne<Model>,
  setData: (data: Snapshot<Model | null>) => void
): void;

export function useQuery<Model extends Record<string, any>>(
  queryResult: ReactiveQueryResult<Model> | ReactiveQueryResultOne<Model>,
  setData:
    | ((data: readonly Snapshot<Model>[]) => void)
    | ((data: Snapshot<Model | null>) => void)
): void {
  // Subscribe to changes in the query result data
  const unsubscribe = subscribe(
    queryResult instanceof ReactiveQueryResultOne
      ? (queryResult as any).state
      : queryResult.data,
    () => {
      if (queryResult instanceof ReactiveQueryResultOne) {
        const data = queryResult.data;
        (setData as any)(data === null ? null : snapshot(data as any));
      } else {
        (setData as any)(snapshot(queryResult.data as any));
      }
    }
  );

  // Initial sync to ensure we have the latest data
  createEffect(() => {
    if (queryResult instanceof ReactiveQueryResultOne) {
      const data = queryResult.data;
      (setData as any)(data === null ? null : snapshot(data as any));
    } else {
      (setData as any)(snapshot(queryResult.data as any));
    }
  });

  // Clean up subscription when component unmounts
  onCleanup(() => {
    unsubscribe();
    queryResult.kill();
  });
}
