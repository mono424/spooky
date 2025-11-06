import {
  Context,
  Effect,
  Layer,
  Logger,
  LogLevel,
  Runtime,
  Schedule,
} from "effect";
import {
  SchemaStructure,
  TableModel,
  RecordId,
  TableNames,
  GetTable,
} from "@spooky/query-builder";
import { DatabaseService, makeConfig } from "./index.js";
import { encodeToSpooky, generateNewId } from "./converter.js";

export type MutationType = "create" | "update" | "delete";

export type Mutation<S extends SchemaStructure, N extends TableNames<S>> =
  | CreateMutation<S, N>
  | UpdateMutation<S, N>
  | DeleteMutation<S, N>;

export interface JsonPatch {
  op: string;
  path: string;
  value?: unknown;
}

export interface CreateMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "create";
  tableName: N;
  data: TableModel<GetTable<S, N>>;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface UpdateMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "update";
  tableName: N;
  recordId: RecordId;
  patches: JsonPatch[];
  rollbackPatches: JsonPatch[] | null;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface DeleteMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "delete";
  tableName: N;
  recordId: RecordId;
  rollbackData: TableModel<GetTable<S, N>> | null;
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
    run: ReturnType<typeof makeRun>;
    create: <S extends SchemaStructure, N extends TableNames<S>>(
      tableName: N,
      payload: TableModel<GetTable<S, N>>
    ) => Effect.Effect<void, string | Error, never>;
    update: <S extends SchemaStructure, N extends TableNames<S>>(
      tableName: N,
      recordId: RecordId,
      payload: Partial<TableModel<GetTable<S, N>>>
    ) => Effect.Effect<void, string | Error, never>;
    delete: <S extends SchemaStructure, N extends TableNames<S>>(
      tableName: N,
      id: RecordId
    ) => Effect.Effect<void, string | Error, never>;
  }
>() {}

const createMutation = Effect.fn("createMutation")(function* <
  S extends SchemaStructure,
  N extends TableNames<S>
>(mutation: Mutation<S, N>) {
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
  S extends SchemaStructure,
  N extends TableNames<S>
>(mutation: Mutation<S, N>) {
  yield* Effect.logDebug("[MutationManager] Apply locally", {
    id: mutation.id,
  });
  const databaseService = yield* DatabaseService;

  switch (mutation.operationType) {
    case "create":
      yield* databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
        `CREATE ${mutation.tableName} CONTENT $payload`,
        {
          payload: mutation.data,
        }
      );
      break;

    case "update":
      const updateMutation = mutation as UpdateMutation<S, N>;
      yield* databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
        `UPDATE ${updateMutation.recordId.toString()} CONTENT $patches`,
        {
          patches: updateMutation.patches,
        }
      );
      break;

    case "delete":
      const deleteMutation = mutation as DeleteMutation<S, N>;
      yield* databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
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
  S extends SchemaStructure,
  N extends TableNames<S>
>(mutation: Mutation<S, N>) {
  yield* Effect.logDebug("[MutationManager] Apply locally", {
    id: mutation.id,
  });
  const databaseService = yield* DatabaseService;

  switch (mutation.operationType) {
    case "create":
      yield* Effect.logDebug("[MutationManager] Create remote", {
        id: mutation.id,
        data: mutation.data,
      });
      yield* databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
        `CREATE ${mutation.tableName} CONTENT $payload`,
        {
          payload: mutation.data,
        }
      );
      break;

    case "update":
      yield* Effect.logDebug("[MutationManager] Update remote", {
        id: mutation.id,
        recordId: mutation.recordId,
        data: mutation.patches,
      });
      yield* databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
        `UPDATE ${mutation.recordId.toString()} CONTENT $patches`,
        {
          patches: mutation.patches,
        }
      );
      break;

    case "delete":
      yield* Effect.logDebug("[MutationManager] Delete remote", {
        id: mutation.id,
        recordId: mutation.recordId,
      });
      yield* databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
        `DELETE ${mutation.recordId.toString()}`
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
  const run = <S extends SchemaStructure, N extends TableNames<S>>(
    mutation: Mutation<S, N>
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
        Effect.retry({
          times: 3,
          schedule: Schedule.exponential("10 millis"),
        }),
        Effect.catchAll((error) =>
          Effect.logError("Failed to apply mutation remotely", error)
        ),
        Effect.provide(runtime)
      );
      yield* Effect.logDebug("[MutationManager] Run - Done");
    }).pipe(Logger.withMinimumLogLevel(LogLevel.Debug));

  return run;
};

export const makeCreate = <S extends SchemaStructure>(
  schema: S,
  run: ReturnType<typeof makeRun>
) => {
  return Effect.fn("create")(function* <N extends TableNames<S>>(
    tableName: N,
    payload: TableModel<GetTable<S, N>>
  ) {
    const encodedPayload = yield* encodeToSpooky(schema, tableName, payload);
    if (!encodedPayload) {
      yield* Effect.fail("payload could not be encoded");
      return;
    }

    const encodedPayloadWithId = yield* generateNewId(
      schema,
      tableName,
      encodedPayload
    );
    if (!encodedPayloadWithId) {
      yield* Effect.fail("id could not be generated");
      return;
    }
    return yield* run<S, N>({
      operationType: "create",
      tableName: tableName,
      data: encodedPayloadWithId,
      createdAt: new Date(),
      retryCount: 0,
    });
  });
};

export const makeUpdate = <S extends SchemaStructure>(
  schema: S,
  run: ReturnType<typeof makeRun>
) => {
  return Effect.fn("update")(function* <N extends TableNames<S>>(
    tableName: N,
    recordId: RecordId,
    payload: Partial<TableModel<GetTable<S, N>>>
  ) {
    const patches = Object.entries(payload)
      .filter(([key]) => key !== "id")
      .map(([key, value]) => ({
        op: "replace",
        path: `/${key}`,
        value: value,
      }));

    return yield* run<S, N>({
      operationType: "update",
      recordId,
      tableName: tableName,
      patches: patches,
      rollbackPatches: null,
      createdAt: new Date(),
      retryCount: 0,
    });
  });
};

export const makeDelete = <S extends SchemaStructure>(
  schema: S,
  run: ReturnType<typeof makeRun>
) => {
  return Effect.fn("delete")(function* <N extends TableNames<S>>(
    tableName: N,
    id: RecordId
  ) {
    return yield* run<S, N>({
      operationType: "delete",
      tableName: tableName,
      recordId: id,
      rollbackData: null,
      createdAt: new Date(),
      retryCount: 0,
    });
  });
};

export const MutationManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    MutationManagerService,
    Effect.gen(function* () {
      const { schema } = yield* (yield* makeConfig<S>()).getConfig;
      const runtime = yield* Effect.runtime<DatabaseService>();
      const run = makeRun(runtime);

      return MutationManagerService.of({
        runtime,
        run,
        create: makeCreate(schema, run),
        update: makeUpdate(schema, run),
        delete: makeDelete(schema, run),
      });
    })
  );
