import { createRemoteEngines, Surreal, Uuid } from "surrealdb";
import { Effect, Layer } from "effect";
import { CacheStrategy, makeConfig } from "./config.js";
import { SchemaStructure } from "@spooky/query-builder";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteDatabaseError,
} from "./database.js";
import { createWasmEngines } from "@surrealdb/wasm";

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

const queryLocalDatabase =
  (db: Surreal) =>
  <T>(sql: string, vars?: Record<string, unknown>) =>
    Effect.tryPromise({
      try: async () => {
        const result = await db.query(sql, vars).collect<[T]>();
        return result[0];
      },
      catch: (error) =>
        new LocalDatabaseError({
          message: "Failed to execute query on local database",
          cause: error,
        }),
    });

const queryRemoteDatabase =
  (db: Surreal) =>
  <T>(sql: string, vars?: Record<string, unknown>) =>
    Effect.tryPromise({
      try: async () => {
        const result = await db.query(sql, vars).collect<[T]>();
        return result[0];
      },
      catch: (error) =>
        new RemoteDatabaseError({
          message: "Failed to execute query on remote database",
          cause: error,
        }),
    });

const liveOfRemoteDatabase = (db: Surreal) => (liveUuid: Uuid) =>
  Effect.tryPromise({
    try: async () => {
      const result = await db.liveOf(liveUuid);
      return result;
    },
    catch: (error) =>
      new RemoteDatabaseError({
        message: "Failed to execute liveOf on remote database",
        cause: error,
      }),
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
        queryLocal: queryLocalDatabase(localDatabase),
        queryInternal: queryLocalDatabase(internalDatabase),
        queryRemote: queryRemoteDatabase(remoteDatabase),
        liveOfRemote: liveOfRemoteDatabase(remoteDatabase),
      });
    })
  );
