import { Context, Effect, Layer } from "effect";
import { makeConfig } from "./config.js";
import {
  ColumnSchema,
  InnerQuery,
  SchemaStructure,
  TableNames,
} from "@spooky/query-builder";
import { DatabaseService } from "./index.js";

export type CleanupFn = () => void;

export interface Query<S extends SchemaStructure, Table extends TableNames<S>> {
  innerQuery: InnerQuery<
    Extract<
      S["tables"][number],
      {
        name: Table;
      }
    >,
    boolean
  >;
  cleanup: CleanupFn;
}

export interface QueryCache<S extends SchemaStructure> {
  [key: number]: Query<S, TableNames<S>[number]>;
}

export class QueryManagerService extends Context.Tag("QueryManagerService")<
  QueryManagerService,
  {
    readonly cache: QueryCache<SchemaStructure>;
    run: <T extends { columns: Record<string, ColumnSchema> }>(
      query: InnerQuery<T, boolean>
    ) => Effect.Effect<CleanupFn, never, never>;
  }
>() {}

const cache: QueryCache<SchemaStructure> = {};

const initQuery = Effect.fn("initQuery")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
  const databaseService = yield* DatabaseService;
  const localDb = yield* databaseService.useLocal((local) =>
    Effect.succeed(local)
  );
});

const run = Effect.fn("run")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(query: InnerQuery<T, boolean>) {
  if (!cache[query.hash]) {
    cache[query.hash] = {
      innerQuery: query,
      cleanup: () => {},
    };
    yield* initQuery(query);
  }
  return cache[query.hash].cleanup;
});

export const QueryManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    QueryManagerService,
    Effect.gen(function* () {
      return QueryManagerService.of({
        cache: cache as QueryCache<S>,
        run,
      });
    })
  );
