import { Context, Effect, Layer, Logger, LogLevel, Runtime } from "effect";
import {
  ColumnSchema,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import { DatabaseService } from "./index.js";
import { RecordId } from "surrealdb";

export type MutationType = "create" | "update" | "delete";

export type Mutation<T extends Record<string, unknown>> =
  | CreateMutation<T>
  | UpdateMutation<T>
  | DeleteMutation<T>;

export interface JsonPatch {
  op: string;
  path: string;
  value?: unknown;
}

export interface CreateMutation<T extends Record<string, unknown>> {
  id?: RecordId;
  operationType: "create";
  tableName: string;
  data: T;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface UpdateMutation<T extends Record<string, unknown>> {
  id?: RecordId;
  operationType: "update";
  tableName: string;
  patches: JsonPatch[];
  rollbackPatches: JsonPatch[];
  recordId: RecordId;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface DeleteMutation<T extends Record<string, unknown>> {
  id?: RecordId;
  operationType: "delete";
  tableName: string;
  recordId: RecordId;
  rollbackData: T;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export class MutationManagerService extends Context.Tag(
  "MutationManagerService"
)<
  MutationManagerService,
  {
    readonly runtime: Runtime.Runtime<DatabaseService>;
    run: <T extends { columns: Record<string, ColumnSchema> }>(
      mutation: Mutation<T>
    ) => Effect.Effect<void, never, never>;
  }
>() {}

const createMutation = Effect.fn("createMutation")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(mutation: Mutation<Record<string, unknown>>) {
  const databaseService = yield* DatabaseService;
  const [result] = yield* databaseService.queryInternal<{ id: RecordId }[]>(
    `CREATE _mutations CONTENT $payload`,
    {
      payload: mutation,
    }
  );
  return {
    ...mutation,
    id: result.id,
  };
});

const mutationApplyLocal = Effect.fn("queryLocalRefresh")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(mutation: Mutation<Record<string, unknown>>) {
  yield* Effect.logDebug("[MutationManager] Apply locally", {
    id: mutation.id,
  });
  const databaseService = yield* DatabaseService;

  switch (mutation.operationType) {
    case "create":
      yield* databaseService.queryLocal<TableModel<T>[]>(
        `CREATE ${mutation.tableName} CONTENT $payload`,
        {
          payload: mutation.data,
        }
      );
      break;

    case "update":
      const updateMutation = mutation as UpdateMutation<
        Record<string, unknown>
      >;
      yield* databaseService.queryLocal<TableModel<T>[]>(
        `UPDATE ${updateMutation.recordId.toString()} CONTENT $patches`,
        {
          patches: updateMutation.patches,
        }
      );
      break;

    case "delete":
      const deleteMutation = mutation as DeleteMutation<
        Record<string, unknown>
      >;
      yield* databaseService.queryLocal<TableModel<T>[]>(
        `DELETE ${deleteMutation.recordId.toString()}`
      );
      break;

    default:
      yield* Effect.die(`Unknown mutation type`);
      return;
  }

  yield* Effect.logDebug("[MutationManager] Apply locally - Done", {
    id: mutation.id,
  });
});

const mutationApplyRemote = Effect.fn("queryLocalRefresh")(function* <
  T extends { columns: Record<string, ColumnSchema> }
>(mutation: Mutation<Record<string, unknown>>) {
  yield* Effect.logDebug("[MutationManager] Apply locally", {
    id: mutation.id,
  });
  const databaseService = yield* DatabaseService;

  switch (mutation.operationType) {
    case "create":
      yield* databaseService.queryRemote<TableModel<T>[]>(
        `CREATE ${mutation.tableName} CONTENT $payload`,
        {
          payload: mutation.data,
        }
      );
      break;

    case "update":
      const updateMutation = mutation as UpdateMutation<
        Record<string, unknown>
      >;
      yield* databaseService.queryRemote<TableModel<T>[]>(
        `UPDATE ${updateMutation.recordId.toString()} CONTENT $patches`,
        {
          patches: updateMutation.patches,
        }
      );
      break;

    case "delete":
      const deleteMutation = mutation as DeleteMutation<
        Record<string, unknown>
      >;
      yield* databaseService.queryRemote<TableModel<T>[]>(
        `DELETE ${deleteMutation.recordId.toString()}`
      );
      break;

    default:
      yield* Effect.die(`Unknown mutation type`);
      return;
  }

  yield* Effect.logDebug("[MutationManager] Apply locally - Done", {
    id: mutation.id,
  });
});

const makeRun = (runtime: Runtime.Runtime<DatabaseService>) => {
  const run = <T extends { columns: Record<string, ColumnSchema> }>(
    mutation: Mutation<T>
  ) =>
    Effect.gen(function* () {
      yield* Effect.logDebug("[MutationManager] Run - Starting");
      yield* createMutation(mutation).pipe(
        Effect.catchAll((error) =>
          Effect.logError("Failed to create mutation", error)
        ),
        Effect.provide(runtime)
      );
      yield* mutationApplyLocal(mutation).pipe(
        Effect.catchAll((error) =>
          Effect.logError("Failed to apply mutation locally", error)
        ),
        Effect.provide(runtime)
      );
      yield* mutationApplyRemote(mutation).pipe(
        Effect.catchAll((error) =>
          Effect.logError("Failed to apply mutation remotely", error)
        ),
        Effect.provide(runtime)
      );
      yield* Effect.logDebug("[MutationManager] Run - Done");
    }).pipe(Logger.withMinimumLogLevel(LogLevel.Debug));

  return run;
};

export const MutationManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    MutationManagerService,
    Effect.gen(function* () {
      const runtime = yield* Effect.runtime<DatabaseService>();
      const run = makeRun(runtime);

      return MutationManagerService.of({
        runtime,
        run,
      });
    })
  );
