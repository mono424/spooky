import { Console, Context, Effect, Layer, Runtime } from "effect";
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
  const databaseService = yield* DatabaseService;

  const [result] = yield* databaseService.queryLocal<TableModel<T>[]>(
    query.selectQuery.query,
    query.selectQuery.vars
  );

  query.setData(result);
});

const handleRemoteUpdate = Effect.fn("handleRemoteUpdate")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>, event: LiveMessage) {
  switch (event.action) {
    case "CREATE":
      yield* Console.log("Created:", event.value);
      break;
    case "UPDATE":
      yield* Console.log("Updated:", event.value);
      break;
    case "DELETE":
      yield* Console.log("Deleted:", event.value);
      break;
    default:
      yield* Effect.fail("failed to handle remote update");
  }
});

const subscribeRemoteQuery = Effect.fn("subscribeRemoteQuery")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
  const databaseService = yield* DatabaseService;
  const [liveUuid] = yield* databaseService.queryRemote<Uuid>(
    query.selectLiveQuery.query,
    query.selectLiveQuery.vars
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
});

const makeRun = (runtime: Runtime.Runtime<DatabaseService>) => {
  const run = <T extends { columns: Record<string, ColumnSchema> }>(
    query: InnerQuery<T, boolean>
  ): Effect.Effect<{ cleanup: CleanupFn }, never, never> =>
    Effect.gen(function* () {
      if (!cache[query.hash]) {
        cache[query.hash] = {
          innerQuery: query,
          cleanup: () => {},
        };

        yield* Effect.fork(
          Effect.gen(function* () {
            yield* queryLocalRefresh(query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to refresh query locally", error)
              ),
              Effect.provide(runtime)
            );
            yield* subscribeRemoteQuery(query).pipe(
              Effect.catchAll((error) =>
                Effect.logError("Failed to subscribe to remote query", error)
              ),
              Effect.provide(runtime)
            );

            for (const subquery of query.subqueries) {
              const unsubscribe = subquery.subscribe(() => {
                Runtime.runPromise(runtime)(
                  queryLocalRefresh(query).pipe(
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
      const runtime = yield* Effect.runtime<DatabaseService>();
      const run = makeRun(runtime);

      return QueryManagerService.of({
        runtime,
        cache: cache as QueryCache<S>,
        run,
      });
    })
  );
