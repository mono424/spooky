// spooky.ts
import { Effect } from "effect";
import {
  Executor,
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { makeConfig } from "./services/index.js";
import { provision } from "./provision.js";

const create = Effect.fn("create")(function* (table: string, data: any) {
  return yield* Effect.succeed(data);
});

const update = Effect.fn("update")(function* (table: string, data: any) {
  return yield* Effect.succeed(data);
});

const deleteFn = Effect.fn("delete")(function* (table: string, data: any) {
  return yield* Effect.succeed(data);
});

const useQuery =
  <S extends SchemaStructure>(schema: S) =>
  <Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>
  ) =>
    Effect.succeed(
      new QueryBuilder<S, Table>(
        schema,
        table,
        (query) => {
          return {
            cleanup: () => {},
          };
        },
        options
      )
    );

// spooky.ts
export const main = <S extends SchemaStructure>() =>
  Effect.gen(function* () {
    const { schema } = yield* (yield* makeConfig<S>()).getConfig;

    yield* provision<S>();

    return {
      create,
      update,
      delete: deleteFn,
      query: useQuery<S>(schema),
    };
  });
