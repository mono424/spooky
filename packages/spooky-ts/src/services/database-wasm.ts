import { createRemoteEngines, Surreal } from "surrealdb";
import { Effect, Layer } from "effect";
import { CacheStrategy, makeConfig } from "./config.js";
import { SchemaStructure } from "@spooky/query-builder";
import {
  DatabaseService,
  LocalDatabaseError,
  makeAuthenticateRemoteDatabase,
  makeClearLocalCache,
  makeCloseLocalDatabase,
  makeCloseRemoteDatabase,
  makeDeauthenticateRemoteDatabase,
  makeLiveOfRemoteDatabase,
  makeQueryLocalDatabase,
  makeQueryRemoteDatabase,
  makeUseLocalDatabase,
  makeUseRemoteDatabase,
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
        useLocal: makeUseLocalDatabase(localDatabase),
        useInternal: makeUseLocalDatabase(internalDatabase),
        useRemote: makeUseRemoteDatabase(remoteDatabase),
        queryLocal: makeQueryLocalDatabase(localDatabase),
        queryInternal: makeQueryLocalDatabase(internalDatabase),
        queryRemote: makeQueryRemoteDatabase(remoteDatabase),
        liveOfRemote: makeLiveOfRemoteDatabase(remoteDatabase),
        authenticate: makeAuthenticateRemoteDatabase(remoteDatabase),
        deauthenticate: makeDeauthenticateRemoteDatabase(remoteDatabase),
        closeRemote: makeCloseRemoteDatabase(remoteDatabase),
        closeLocal: makeCloseLocalDatabase(localDatabase),
        closeInternal: makeCloseLocalDatabase(internalDatabase),
        clearLocalCache: makeClearLocalCache(localDatabase),
      });
    })
  );
