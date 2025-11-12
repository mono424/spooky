import { Context, Effect, Layer, Runtime } from "effect";
import {
  ColumnSchema,
  InnerQuery,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import { DatabaseService, makeConfig } from "./index.js";
import { LiveMessage, Uuid } from "surrealdb";
import { decodeFromSpooky } from "./converter.js";

export type CleanupFn = () => void;

export interface Query<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
> {
  innerQuery: InnerQuery<T, IsOne>;
  cleanup: CleanupFn;
}

export interface QueryCache<S extends SchemaStructure> {
  [key: number]: Query<{ columns: S["tables"][number]["columns"] }, boolean>;
}

export class QueryManagerService extends Context.Tag("QueryManagerService")<
  QueryManagerService,
  {
    readonly runtime: Runtime.Runtime<DatabaseService>;
    readonly cache: QueryCache<SchemaStructure>;
    run: <T extends { columns: Record<string, ColumnSchema> }>(
      query: InnerQuery<T, boolean>
    ) => ReturnType<ReturnType<typeof makeRun>>;
    refreshTableQueries: <T extends { columns: Record<string, ColumnSchema> }>(
      table: string
    ) => ReturnType<ReturnType<typeof makeRefreshTableQueries>>;
  }
>() {}

const cache: QueryCache<SchemaStructure> = {};

/**
 * Custom JSON.stringify function that formats Date objects as SurrealDB date literals.
 * Dates are formatted as: d"2025-11-12T05:47:42.527106262Z"
 */
function surrealStringify<S extends SchemaStructure>(
  schema: S,
  tableName: string,
  value: unknown
): string {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    throw new Error(`Table ${tableName} not found in schema`);
  }

  // Use a unique placeholder for dates to avoid conflicts
  const DATE_PLACEHOLDER = "__SURREAL_DATE__";

  // First pass: replace Date objects with placeholders
  const replacer = (key: string, val: unknown): unknown => {
    if (table.columns[key]?.dateTime) {
      return DATE_PLACEHOLDER + val + DATE_PLACEHOLDER;
    }
    return val;
  };

  // Stringify with replacer
  let jsonString = JSON.stringify(value, replacer);
  // Second pass: replace placeholders with SurrealDB date format
  // Match the placeholder pattern and replace with SurrealDB date literal
  jsonString = jsonString.replace(
    new RegExp(`"${DATE_PLACEHOLDER}([^"]+)${DATE_PLACEHOLDER}"`, "g"),
    (match, isoString) => `d"${isoString}"`
  );

  return jsonString;
}

const queryLocalRefresh = Effect.fn("queryLocalRefresh")(function* <
  S extends SchemaStructure,
  T extends { columns: Record<string, ColumnSchema> }
>(schema: S, query: InnerQuery<T, boolean>) {
  yield* Effect.logDebug("[QueryManager] Local Query Refresh - Starting", {
    queryHash: query.hash,
  });
  const databaseService = yield* DatabaseService;

  const results = yield* databaseService.queryLocal<TableModel<T>[]>(
    query.selectQuery.query,
    query.selectQuery.vars
  );
  yield* Effect.logDebug("[QueryManager] Local Query Refresh - Done", {
    queryHash: query.hash,
    resultLength: results?.length ?? 0,
  });

  const decodedResults = yield* Effect.all(
    results.map((result) => decodeFromSpooky(schema, query.tableName, result))
  );

  query.setData(decodedResults.filter((result) => result !== undefined));
});

const makeRefreshTableQueries = <S extends SchemaStructure>(schema: S) =>
  Effect.fn("refreshTableQueries")(function* <
    T extends { columns: Record<string, ColumnSchema> }
  >(table: string) {
    yield* Effect.logDebug("[QueryManager] Refresh Table Queries - Starting", {
      table,
    });

    for (const query of Object.values(cache)) {
      if (query.innerQuery.tableName === table) {
        yield* queryLocalRefresh(schema, query.innerQuery);
      }
    }

    yield* Effect.logDebug("[QueryManager] Refresh Table Queries - Done", {
      table,
    });
  });

const queryRemoteHydration = Effect.fn("queryRemoteHydration")(function* <
  S extends SchemaStructure,
  T extends { columns: Record<string, ColumnSchema> }
>(schema: S, query: InnerQuery<T, boolean>) {
  yield* Effect.logDebug("[QueryManager] Remote Query Hydration - Starting", {
    queryHash: query.hash,
    query: query.selectQuery.query,
  });

  const databaseService = yield* DatabaseService;
  const results = yield* databaseService.queryRemote<TableModel<T>[]>(
    query.selectQuery.query,
    query.selectQuery.vars
  );

  yield* Effect.logDebug(
    "[QueryManager] Remote Query Hydration - Remote query done",
    {
      queryHash: query.hash,
      resultLength: results?.length ?? 0,
    }
  );

  const hydrateQuery = results
    .map(
      ({ id, ...payload }) =>
        `UPSERT ${id} CONTENT ${surrealStringify(
          schema,
          query.tableName,
          payload
        )}`
    )
    .join(";\n");

  yield* Effect.logDebug(
    "[QueryManager] Remote Query Hydration - Updating local cache",
    {
      queryHash: query.hash,
      hydrateQuery,
    }
  );

  yield* databaseService.queryLocal(hydrateQuery);

  yield* Effect.logDebug(
    "[QueryManager] Remote Query Hydration - Local cache updated",
    {
      queryHash: query.hash,
      resultLength: results?.length ?? 0,
    }
  );

  const decodedResults = yield* Effect.all(
    results.map((result) => decodeFromSpooky(schema, query.tableName, result))
  );

  query.setData(decodedResults.filter((result) => result !== undefined));
});

const handleRemoteUpdate = Effect.fn("handleRemoteUpdate")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>, event: LiveMessage) {
  switch (event.action) {
    case "CREATE":
      yield* Effect.logDebug(
        "[QueryManager] Live Event - Created:",
        event.value
      );
      break;
    case "UPDATE":
      yield* Effect.logDebug(
        "[QueryManager] Live Event - Updated:",
        event.value
      );
      break;
    case "DELETE":
      yield* Effect.logDebug(
        "[QueryManager] Live Event - Deleted:",
        event.value
      );
      break;
    default:
      yield* Effect.logError(
        "[QueryManager] Live Event - failed to handle remote update",
        event
      );
  }
});

const subscribeRemoteQuery = Effect.fn("subscribeRemoteQuery")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
  yield* Effect.logDebug("[QueryManager] Subscribe Remote Query - Starting", {
    queryHash: query.hash,
    query: query.selectLiveQuery.query,
  });
  const databaseService = yield* DatabaseService;
  const [liveUuid] = yield* databaseService.queryRemote<Uuid[]>(
    query.selectLiveQuery.query,
    query.selectLiveQuery.vars
  );

  yield* Effect.logDebug(
    "[QueryManager] Subscribe Remote Query - Created Live UUID",
    {
      queryHash: query.hash,
      liveUuid: liveUuid,
    }
  );

  const runtime = yield* Effect.runtime<DatabaseService>();

  const subscription = yield* databaseService.liveOfRemote(liveUuid);
  subscription.subscribe(async (event: LiveMessage) =>
    Runtime.runPromise(runtime)(
      handleRemoteUpdate(query, event).pipe(
        Effect.catchAll((error) =>
          Effect.logError("Failed to refresh query after subscription", error)
        )
      )
    )
  );

  yield* Effect.logDebug(
    "[QueryManager] Subscribe Remote Query - Subscribed to Live UUID",
    {
      queryHash: query.hash,
      liveUuid: liveUuid,
    }
  );
});

const makeRun = <S extends SchemaStructure>(
  schema: S,
  runtime: Runtime.Runtime<DatabaseService>
) => {
  const run = <T extends { columns: Record<string, ColumnSchema> }>(
    query: InnerQuery<T, boolean>
  ): Effect.Effect<{ cleanup: CleanupFn }, never, never> =>
    Effect.gen(function* () {
      yield* Effect.logDebug("[QueryManager] Run - Starting", {
        queryHash: query.hash,
      });

      if (!cache[query.hash]) {
        yield* Effect.logDebug("[QueryManager] Run - Cache miss", {
          queryHash: query.hash,
        });

        cache[query.hash] = {
          innerQuery: query,
          cleanup: () => {},
        };

        Effect.runFork(
          Effect.gen(function* () {
            yield* Effect.logDebug("[QueryManager] Run - Initialize query", {
              queryHash: query.hash,
            });

            yield* Effect.logDebug(
              "[QueryManager] Run - Refresh query locally",
              { queryHash: query.hash }
            );

            yield* queryLocalRefresh(schema, query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to refresh query locally", error)
              ),
              Effect.provide(runtime)
            );

            yield* Effect.logDebug(
              "[QueryManager] Run - Hydrate remote query",
              { queryHash: query.hash }
            );

            yield* queryRemoteHydration(schema, query).pipe(
              Effect.catchAll((error) =>
                Effect.gen(function* () {
                  yield* Effect.logWarning(
                    "[QueryManager] Remote hydration failed (continuing with local data)",
                    error
                  );
                  return Effect.succeed(undefined);
                })
              ),
              Effect.provide(runtime)
            );

            yield* Effect.logDebug(
              "[QueryManager] Run - Subscribe to remote query",
              { queryHash: query.hash }
            );
            yield* subscribeRemoteQuery(query).pipe(
              Effect.catchAll((error) =>
                Effect.gen(function* () {
                  yield* Effect.logWarning(
                    "[QueryManager] Remote subscription failed (continuing with local data)",
                    error
                  );
                  return Effect.succeed(undefined);
                })
              ),
              Effect.provide(runtime)
            );

            yield* Effect.logDebug(
              "[QueryManager] Run - Initialize subqueries",
              query.hash
            );
            for (const subquery of query.subqueries) {
              const unsubscribe = subquery.subscribe(() => {
                Runtime.runPromise(runtime)(
                  queryLocalRefresh(schema, query).pipe(
                    Effect.catchAll((error) =>
                      Effect.logError(
                        "Failed to refresh query after subscription",
                        error
                      )
                    )
                  )
                );
              });

              const previousCleanup = cache[query.hash].cleanup;
              cache[query.hash].cleanup = () => {
                previousCleanup();
                unsubscribe();
              };

              yield* run(subquery);
            }
          })
        );
      } else {
        yield* Effect.logDebug("[QueryManager] Run - Cache hit", query.hash);
      }

      return {
        cleanup: cache[query.hash].cleanup,
      };
    });

  return run;
};

export const QueryManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    QueryManagerService,
    Effect.gen(function* () {
      const { schema } = yield* (yield* makeConfig<S>()).getConfig;

      const runtime = yield* Effect.runtime<DatabaseService>();
      const run = makeRun<S>(schema, runtime);

      const refreshTableQueries = makeRefreshTableQueries<S>(schema);

      return QueryManagerService.of({
        runtime,
        cache: cache as QueryCache<S>,
        refreshTableQueries,
        run,
      });
    })
  );
