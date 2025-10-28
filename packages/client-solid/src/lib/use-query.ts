import { onCleanup, createEffect, Accessor } from "solid-js";
import { ReactiveQueryResult, ReactiveQueryResultOne } from "./table-queries";
import { snapshot, Snapshot, subscribe } from "valtio";

/**
 * A SolidJS hook that subscribes to a ReactiveQueryResult and returns a signal
 *
 * @param queryResult - The reactive query result to subscribe to (can be static or accessor)
 * @param setData - Setter function to update the data signal
 *
 * @example
 * ```tsx
 * // Static query
 * const threadsQuery = db.query.thread
 *   .find({})
 *   .orderBy("created_at", "desc")
 *   .query();
 *
 * const [threads, setThreads] = createSignal([]);
 * useQuery(threadsQuery, setThreads);
 *
 * // Reactive query
 * const threadQuery = () => db.query.thread
 *   .find({ id: params.id })
 *   .related("author")
 *   .one();
 *
 * const [thread, setThread] = createSignal(null);
 * useQuery(threadQuery, setThread);
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
  queryResult: Accessor<ReactiveQueryResult<Model>>,
  setData: (data: readonly Snapshot<Model>[]) => void
): void;

export function useQuery<Model extends Record<string, any>>(
  queryResult: Accessor<ReactiveQueryResultOne<Model>>,
  setData: (data: Snapshot<Model | null>) => void
): void;

export function useQuery<Model extends Record<string, any>>(
  queryResult:
    | ReactiveQueryResult<Model>
    | ReactiveQueryResultOne<Model>
    | Accessor<ReactiveQueryResult<Model>>
    | Accessor<ReactiveQueryResultOne<Model>>,
  setData:
    | ((data: readonly Snapshot<Model>[]) => void)
    | ((data: Snapshot<Model | null>) => void)
): void {
  // Determine if queryResult is an accessor
  const isAccessor = typeof queryResult === "function";

  // Track the current query and cleanup
  createEffect(() => {
    // Get the current query (either directly or from accessor)
    // If it's an accessor, calling it will track any reactive dependencies inside
    const query = isAccessor ? (queryResult as Accessor<any>)() : queryResult;

    console.log("[useQuery] Effect running, got query:", query);

    // Subscribe to changes in the query result data
    const unsubscribe = subscribe(
      query instanceof ReactiveQueryResultOne
        ? (query as any).state
        : (query as any).data,
      () => {
        if (query instanceof ReactiveQueryResultOne) {
          const data = query.data;
          (setData as any)(data === null ? null : snapshot(data as any));
        } else {
          (setData as any)(snapshot((query as any).data as any));
        }
      }
    );

    // Initial sync to ensure we have the latest data
    if (query instanceof ReactiveQueryResultOne) {
      const data = query.data;
      (setData as any)(data === null ? null : snapshot(data as any));
    } else {
      (setData as any)(snapshot((query as any).data as any));
    }

    // Clean up subscription when query changes or component unmounts
    onCleanup(() => {
      unsubscribe();
      query.kill();
    });
  });
}
