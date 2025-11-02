import { createRemoteEngines, Surreal } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { Context, Data, Effect, Layer } from "effect";
import { CacheStrategy, makeConfig } from "./config.js";
import { SchemaStructure } from "@spooky/query-builder";

export class LocalDatabaseError extends Data.TaggedError("LocalDatabaseError")<{
  readonly cause?: unknown;
  readonly message: string;
}> {}

export class RemoteDatabaseError extends Data.TaggedError(
  "RemoteDatabaseError"
)<{
  readonly cause?: unknown;
  readonly message: string;
}> {}

export class DatabaseService extends Context.Tag("DatabaseService")<
  DatabaseService,
  {
    useLocal: <T>(
      fn: (local: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    useInternal: <T>(
      fn: (internal: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    useRemote: <T>(
      fn: (remote: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
    ) => Effect.Effect<T, RemoteDatabaseError, never>;
  }
>() {}

export const connectRemoteDatabase = (url: string) =>
  Effect.tryPromise({
    try: async () => {
      const remote = new Surreal({
        engines: createRemoteEngines(),
        codecOptions: { useNativeDates: false },
      });
      await remote.connect(url);
      return remote;
    },
    catch: (error) =>
      new RemoteDatabaseError({
        message: "Failed to connect to remote database",
        cause: error,
      }),
  });

export const createLocalDatabase = (
  dbName: string,
  strategy: CacheStrategy,
  namespace?: string,
  database?: string
) =>
  Effect.tryPromise({
    try: async () => {
      const local = new Surreal({
        engines: createWasmEngines({
          capabilities: {
            experimental: {
              allow: ["record_references"],
            },
          },
        }),
        codecOptions: {
          useNativeDates: false,
        },
      });

      const connectionUrl =
        strategy === "indexeddb" ? `indxdb://${dbName}` : "mem://";
      await local.connect(connectionUrl);
      await local.use({
        namespace: namespace || "main",
        database: database || dbName,
      });
      return local;
    },
    catch: (error) =>
      new LocalDatabaseError({
        message: "Failed to create local database",
        cause: error,
      }),
  });

const useLocalDatabase =
  (db: Surreal) =>
  <T>(fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>) =>
    Effect.gen(function* () {
      const result = yield* Effect.try({
        try: () => fn(db),
        catch: (error) =>
          new LocalDatabaseError({
            message: "Failed to use database [sync]",
            cause: error,
          }),
      });
      if (result instanceof Promise) {
        return yield* Effect.tryPromise({
          try: () => result,
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to use database [async]",
              cause: error,
            }),
        });
      } else {
        return result;
      }
    });

const useRemoteDatabase =
  (db: Surreal) =>
  <T>(fn: (db: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>) =>
    Effect.gen(function* () {
      const result = yield* Effect.try({
        try: () => fn(db),
        catch: (error) =>
          new RemoteDatabaseError({
            message: "Failed to use database [sync]",
            cause: error,
          }),
      });
      if (result instanceof Promise) {
        return yield* Effect.tryPromise({
          try: () => result,
          catch: (error) =>
            new RemoteDatabaseError({
              message: "Failed to use database [async]",
              cause: error,
            }),
        });
      } else {
        return result;
      }
    });

let localDatabase: Surreal | undefined;
let internalDatabase: Surreal | undefined;
let remoteDatabase: Surreal | undefined;

export const DatabaseServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    DatabaseService,
    Effect.gen(function* () {
      const { localDbName, storageStrategy, namespace, database, remoteUrl } =
        yield* (yield* makeConfig<S>()).getConfig;

      internalDatabase = yield* Effect.acquireRelease(
        createLocalDatabase(localDbName, storageStrategy, "internal", "main"),
        (db) => Effect.promise(() => db.close())
      );

      localDatabase = yield* Effect.acquireRelease(
        createLocalDatabase(localDbName, storageStrategy, namespace, database),
        (db) => Effect.promise(() => db.close())
      );

      remoteDatabase = yield* Effect.acquireRelease(
        connectRemoteDatabase(remoteUrl),
        (db) => Effect.promise(() => db.close())
      );

      return DatabaseService.of({
        useLocal: useLocalDatabase(localDatabase),
        useInternal: useLocalDatabase(internalDatabase),
        useRemote: useRemoteDatabase(remoteDatabase),
      });
    })
  );
