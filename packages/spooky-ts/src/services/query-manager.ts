import { Context, Effect, Layer, Logger, LogLevel, Runtime } from "effect";
import {
  ColumnSchema,
  InnerQuery,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import { DatabaseService } from "./index.js";
import { LiveMessage, Uuid } from "surrealdb";

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
    ) => Effect.Effect<{ cleanup: CleanupFn }, never, never>;
  }
>() {}

const cache: QueryCache<SchemaStructure> = {};

const queryLocalRefresh = Effect.fn("queryLocalRefresh")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
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
  query.setData(results);
});

const queryRemoteHydration = Effect.fn("queryRemoteHydration")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
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
      ({ id, ...payload }) => `UPSERT ${id} CONTENT ${JSON.stringify(payload)}`
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

  query.setData(results);
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
      handleRemoteUpdate(query, event)
        .pipe(
          Effect.catchAll((error) =>
            Effect.logError("Failed to refresh query after subscription", error)
          )
        )
        .pipe(Logger.withMinimumLogLevel(LogLevel.Debug))
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

const makeRun = (runtime: Runtime.Runtime<DatabaseService>) => {
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

            yield* queryLocalRefresh(query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to refresh query locally", error)
              ),
              Effect.provide(runtime)
            );

            yield* Effect.logDebug(
              "[QueryManager] Run - Hydrate remote query",
              { queryHash: query.hash }
            );

            yield* queryRemoteHydration(query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to refresh query locally", error)
              ),
              Effect.provide(runtime)
            );

            yield* Effect.logDebug(
              "[QueryManager] Run - Subscribe to remote query",
              { queryHash: query.hash }
            );
            yield* subscribeRemoteQuery(query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to subscribe to remote query", error)
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
                  queryLocalRefresh(query)
                    .pipe(
                      Effect.catchAll((error) =>
                        Effect.logError(
                          "Failed to refresh query after subscription",
                          error
                        )
                      )
                    )
                    .pipe(Logger.withMinimumLogLevel(LogLevel.Debug))
                );
              });

              const previousCleanup = cache[query.hash].cleanup;
              cache[query.hash].cleanup = () => {
                previousCleanup();
                unsubscribe();
              };

              yield* run(subquery);
            }
          }).pipe(Logger.withMinimumLogLevel(LogLevel.Debug))
        ).pipe(Logger.withMinimumLogLevel(LogLevel.Debug));
      } else {
        yield* Effect.logDebug("[QueryManager] Run - Cache hit", query.hash);
      }

      return {
        cleanup: cache[query.hash].cleanup,
      };
    }).pipe(Logger.withMinimumLogLevel(LogLevel.Debug));

  return run;
};

export const QueryManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    QueryManagerService,
    Effect.gen(function* () {
      const runtime = yield* Effect.runtime<DatabaseService>();
      const run = makeRun(runtime);

      return QueryManagerService.of({
        runtime,
        cache: cache as QueryCache<S>,
        run,
      });
    })
  );
