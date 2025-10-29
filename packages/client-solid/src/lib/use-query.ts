import { onCleanup, createEffect, Accessor, createSignal } from "solid-js";
import { ReactiveQueryResult, ReactiveQueryResultOne } from "./table-queries";
import { snapshot, Snapshot, subscribe } from "valtio";
import { GenericModel } from "./models";

/**
 * A SolidJS hook that subscribes to a ReactiveQueryResult and returns an accessor to the data
 *
 * @param queryResult - The reactive query result to subscribe to (can be static or accessor)
 * @returns Accessor function that returns the current data
 *
 * @example
 * ```tsx
 * // Static query
 * const threadsQuery = db.query.thread
 *   .find({})
 *   .orderBy("created_at", "desc")
 *   .query();
 *
 * const threads = useQuery(threadsQuery);
 *
 * // Reactive query
 * const threadQuery = () => db.query.thread
 *   .find({ id: params.id })
 *   .related("author")
 *   .one();
 *
 * const thread = useQuery(threadQuery);
 * ```
 */

type QueryResult<Model extends GenericModel> =
  | ReactiveQueryResult<Model>
  | ReactiveQueryResultOne<Model>;

// Overload for ReactiveQueryResultOne
export function useQuery<Model extends Record<string, any>>(
  queryResult: ReactiveQueryResultOne<Model> | Accessor<ReactiveQueryResultOne<Model>>
): Accessor<Snapshot<Model> | null>;

// Overload for ReactiveQueryResult
export function useQuery<Model extends Record<string, any>>(
  queryResult: ReactiveQueryResult<Model> | Accessor<ReactiveQueryResult<Model>>
): Accessor<readonly Snapshot<Model>[]>;

// Implementation
export function useQuery<Model extends Record<string, any>>(
  queryResult: QueryResult<Model> | Accessor<QueryResult<Model>>
): Accessor<readonly Snapshot<Model>[] | Snapshot<Model> | null> {
  // Determine if queryResult is an accessor
  const isAccessor = typeof queryResult === "function";

  // Create internal signal to store data
  const [data, setData] = createSignal<readonly Snapshot<Model>[] | Snapshot<Model> | null>(null);

  // Track the previous query to detect changes
  let previousQuery: QueryResult<Model> | null = null;

  // Track the current query and cleanup
  createEffect(() => {
    // Get the current query (either directly or from accessor)
    // If it's an accessor, calling it will track any reactive dependencies inside
    const query = isAccessor ? (queryResult as Accessor<any>)() : queryResult;

    console.log("[useQuery] Effect running, got query:", query);

    // If the query hasn't changed (same object reference), don't re-subscribe
    // This prevents killing and recreating subscriptions unnecessarily
    if (query === previousQuery) {
      console.log("[useQuery] Query unchanged, skipping re-subscription");
      return;
    }

    // Kill the previous query if it's different from the current one
    if (previousQuery !== null && previousQuery !== query) {
      console.log("[useQuery] Query changed, killing previous query");
      previousQuery.kill();
    }

    previousQuery = query;

    // Check if query is a ReactiveQueryResultOne
    const isOne = query instanceof ReactiveQueryResultOne;

    // Subscribe to changes in the query result data
    const unsubscribe = subscribe(query.data, () => {
      if (isOne) {
        // For ReactiveQueryResultOne, extract the value property
        const value = (query.data as { value: any }).value;
        setData(value === null ? null : snapshot(value));
      } else {
        // For ReactiveQueryResult, use the array directly
        setData(snapshot(query.data));
      }
    });

    // Initial sync to ensure we have the latest data
    if (isOne) {
      const value = (query.data as { value: any }).value;
      setData(value === null ? null : snapshot(value));
    } else {
      setData(snapshot(query.data));
    }

    // Clean up subscription when component unmounts
    onCleanup(() => {
      unsubscribe();
      // Note: Don't kill the query here, it will be killed when it changes or on final cleanup
    });
  });

  return data as any;
}
