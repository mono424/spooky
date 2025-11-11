import { createRemoteEngines, RecordId, Surreal, Uuid } from "surrealdb";
import { Effect, Layer } from "effect";
import { CacheStrategy, makeConfig } from "./config.js";
import { SchemaStructure } from "@spooky/query-builder";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteAuthenticationError,
  RemoteDatabaseError,
} from "./database.js";
import { createWasmEngines } from "@surrealdb/wasm";

export const connectRemoteDatabase = Effect.fn("connectRemoteDatabase")(
  function* (url: string) {
    return yield* Effect.tryPromise({
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
  }
);

export const createLocalDatabase = Effect.fn("createLocalDatabase")(function* (
  dbName: string,
  strategy: CacheStrategy,
  namespace?: string,
  database?: string
) {
  return yield* Effect.tryPromise({
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
});

const useLocalDatabase = Effect.fn("useLocalDatabase")(function* (db: Surreal) {
  return Effect.fn("useLocalDatabaseInner")(function* <T>(
    fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
  ) {
    return yield* Effect.gen(function* () {
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
  });
});

const useRemoteDatabase = Effect.fn("useRemoteDatabase")(function* (
  db: Surreal
) {
  return Effect.fn("useRemoteDatabaseInner")(
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
      })
  );
});

const queryLocalDatabase = Effect.fn("queryLocalDatabase")(function* (
  db: Surreal
) {
  return Effect.fn("queryLocalDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    return yield* Effect.tryPromise({
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
  });
});

const queryRemoteDatabase = Effect.fn("queryRemoteDatabase")(function* (
  db: Surreal
) {
  return Effect.fn("queryRemoteDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    return yield* Effect.tryPromise({
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
  });
});

const liveOfRemoteDatabase = Effect.fn("liveOfRemoteDatabase")(function* (
  db: Surreal
) {
  return Effect.fn("liveOfRemoteDatabaseInner")(function* (liveUuid: Uuid) {
    return yield* Effect.tryPromise({
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
  });
});

const authenticateRemoteDatabase = (db: Surreal) => {
  return Effect.fn("authenticateRemoteDatabase")(function* (token: string) {
    return yield* Effect.tryPromise({
      try: async () => {
        await db.authenticate(token);
        const [result] = await db
          .query(`SELECT id FROM $auth`)
          .collect<[{ id: RecordId }]>();
        return result?.id;
      },
      catch: (error) =>
        new RemoteAuthenticationError({
          message: "Failed to authenticate on remote database",
          cause: error,
        }),
    });
  });
};

const makeDeauthenticateRemoteDatabase = (db: Surreal) => {
  return Effect.fn("deauthenticateRemoteDatabase")(function* () {
    return yield* Effect.tryPromise({
      try: async () => {
        await db.invalidate();
      },
      catch: (error) =>
        new RemoteDatabaseError({
          message: "Failed to deauthenticate on remote database",
          cause: error,
        }),
    });
  });
};

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
        useLocal: yield* useLocalDatabase(localDatabase),
        useInternal: yield* useLocalDatabase(internalDatabase),
        useRemote: yield* useRemoteDatabase(remoteDatabase),
        queryLocal: yield* queryLocalDatabase(localDatabase),
        queryInternal: yield* queryLocalDatabase(internalDatabase),
        queryRemote: yield* queryRemoteDatabase(remoteDatabase),
        liveOfRemote: yield* liveOfRemoteDatabase(remoteDatabase),
        authenticate: authenticateRemoteDatabase(remoteDatabase),
        deauthenticate: makeDeauthenticateRemoteDatabase(remoteDatabase),
        closeRemote: () =>
          Effect.tryPromise({
            try: async () => {
              if (remoteDatabase) {
                await remoteDatabase.close();
              }
            },
            catch: (error) =>
              new RemoteDatabaseError({
                message: "Failed to close remote database",
                cause: error,
              }),
          }),
        closeLocal: () =>
          Effect.tryPromise({
            try: async () => {
              if (localDatabase) {
                await localDatabase.close();
              }
            },
            catch: (error) =>
              new LocalDatabaseError({
                message: "Failed to close local database",
                cause: error,
              }),
          }),
        closeInternal: () =>
          Effect.tryPromise({
            try: async () => {
              if (internalDatabase) {
                await internalDatabase.close();
              }
            },
            catch: (error) =>
              new LocalDatabaseError({
                message: "Failed to close internal database",
                cause: error,
              }),
          }),
        clearLocalCache: () =>
          Effect.tryPromise({
            try: async () => {
              if (localDatabase) {
                // Simple implementation: just close and reopen the database
                // This effectively clears the cache
                await localDatabase.close();
                localDatabase = await createLocalDatabase(
                  localDbName,
                  storageStrategy,
                  namespace,
                  database
                ).pipe(Effect.runPromise);
              }
            },
            catch: (error) =>
              new LocalDatabaseError({
                message: "Failed to clear local cache",
                cause: error,
              }),
          }),
      });
    })
  );
