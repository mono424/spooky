// spooky.ts
import { Effect, Runtime } from "effect";
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
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
      create: mutationManager.create,
      update: mutationManager.update,
      delete: mutationManager.delete,
      query,
      close,
      clearLocalCache: databaseService.clearLocalCache,
    };
  });
