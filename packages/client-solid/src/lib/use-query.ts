import { FinalQuery } from "@spooky/query-builder";
import {
  QueryClient,
  QueryFunction,
  QueryKey,
  UndefinedInitialDataOptions,
  UseQueryResult,
  useQuery as useTanstackQuery,
  useQueryClient as useTanstackQueryClient,
} from "@tanstack/solid-query";

export function useQuery<
  TQueryFnData = unknown,
  TError = Error,
  TData = TQueryFnData,
  TQueryKey extends QueryKey = readonly unknown[]
>(
  options: UndefinedInitialDataOptions<TQueryFnData, TError, TData, TQueryKey>,
  queryClient?: () => QueryClient
): UseQueryResult<TData, TError> {
  const client = () => (queryClient ? queryClient() : useTanstackQueryClient());
  const wrappedOptions = () => {
    const opts = options();
    if ("_finalQuery" in opts && opts._finalQuery instanceof FinalQuery) {
      const finalQuery = opts._finalQuery;
      // Capture the query client reference synchronously
      const qc = client();
      const queryKey = opts.queryKey;

      opts.queryFn = (async () => {
        const res = await finalQuery.select();

        // Subscribe to updates and update the query cache when data changes
        res.subscribe((data) => {
          console.log("_finalQuery data update", data);
          // Use the captured query client and query key
          qc.setQueryData(queryKey, data as any);
        });

        // Return the initial data
        return res.data;
      }) as unknown as QueryFunction<TQueryFnData, TQueryKey, never>;
    }
    return opts;
  };
  const q = useTanstackQuery(wrappedOptions, client);

  return q;
}
