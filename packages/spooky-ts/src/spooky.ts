// spooky.ts
import { Effect, Layer, Runtime } from "effect";
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { RecordId } from "surrealdb";
import {
  AuthManagerService,
  DatabaseService,
  makeConfig,
  QueryManagerService,
} from "./services/index.js";
import { provision } from "./provision.js";
import { MutationManagerService } from "./services/mutation-manager.js";

const useQuery = Effect.fn("useQuery")(function* <S extends SchemaStructure>(
  schema: S
) {
  const queryManager = yield* QueryManagerService;
  return Effect.fn("useQueryInner")(function* <Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>
  ) {
    return yield* Effect.succeed(
      new QueryBuilder<S, Table>(
        schema,
        table,
        (q) => Runtime.runSync(queryManager.runtime)(queryManager.run(q)),
        options
      )
    );
  });
});

export const main = <S extends SchemaStructure>() =>
  Effect.gen(function* () {
    const { schema, provisionOptions } = yield* (yield* makeConfig<S>())
      .getConfig;

    yield* provision<S>(provisionOptions);

    const databaseService = yield* DatabaseService;
    const authManager = yield* AuthManagerService;
    const mutationManager = yield* MutationManagerService;
    const queryManager = yield* QueryManagerService;
    const query = yield* useQuery<S>(schema);

    const close = Effect.fn("close")(function* () {
      return yield* Effect.gen(function* () {
        yield* databaseService.closeRemote();
        yield* databaseService.closeLocal();
        yield* databaseService.closeInternal();
      });
    });

    return {
      authenticate: authManager.authenticate,
      deauthenticate: authManager.deauthenticate,
      create: <N extends TableNames<S>>(
        tableName: N,
        payload: TableModel<GetTable<S, N>>
      ) =>
        mutationManager
          .create(tableName, payload)
          .pipe(
            Effect.provide(Layer.succeed(DatabaseService, databaseService)),
            Effect.provide(Layer.succeed(QueryManagerService, queryManager))
          ),
      update: <N extends TableNames<S>>(
        tableName: N,
        recordId: RecordId,
        payload: Partial<TableModel<GetTable<S, N>>>
      ) =>
        mutationManager
          .update(tableName, recordId, payload)
          .pipe(
            Effect.provide(Layer.succeed(DatabaseService, databaseService)),
            Effect.provide(Layer.succeed(QueryManagerService, queryManager))
          ),
      delete: <N extends TableNames<S>>(tableName: N, id: RecordId) =>
        mutationManager
          .delete(tableName, id)
          .pipe(
            Effect.provide(Layer.succeed(DatabaseService, databaseService)),
            Effect.provide(Layer.succeed(QueryManagerService, queryManager))
          ),
      query,
      close,
      clearLocalCache: databaseService.clearLocalCache,
      useRemote: databaseService.useRemote,
    };
  });
