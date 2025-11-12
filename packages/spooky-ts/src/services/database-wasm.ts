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
    yield* Effect.logDebug(
      `[DatabaseService] Connecting to remote database...`
    );

    const startTime = yield* Effect.sync(() => performance.now());

    const remote = yield* Effect.tryPromise({
      try: async () => {
        const r = new Surreal({
          engines: createRemoteEngines(),
          codecOptions: { useNativeDates: false },
        });
        await r.connect(url);
        return r;
      },
      catch: (error) => {
        return new RemoteDatabaseError({
          message: "Failed to connect to remote database",
          cause: error,
        });
      },
    });

    const endTime = yield* Effect.sync(() => performance.now());
    const duration = (endTime - startTime).toFixed(2);

    yield* Effect.logInfo(
      `[DatabaseService] ✅ Connected successfully! (${duration}ms)`
    );
    yield* Effect.logDebug(`[DatabaseService] URL: ${url}`);
    yield* Effect.logInfo(
      `[DatabaseService] ✅ Connected successfully to remote database`
    );

    return remote;
  }
);

export const createLocalDatabase = Effect.fn("createLocalDatabase")(function* (
  dbName: string,
  strategy: CacheStrategy,
  namespace?: string,
  database?: string
) {
  yield* Effect.logDebug(`[DatabaseService] Creating WASM Surreal instance...`);
  yield* Effect.logDebug(`[DatabaseService] Storage strategy: ${strategy}`);
  yield* Effect.logDebug(`[DatabaseService] DB Name: ${dbName}`);

  const local = yield* Effect.tryPromise({
    try: async () => {
      const instance = new Surreal({
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

      // Determine connection URL based on storage strategy
      let connectionUrl: string;
      if (strategy === "indexeddb") {
        // Using indxdb:// protocol for IndexedDB storage
        connectionUrl = `indxdb://${dbName}`;
      } else {
        // Using mem:// protocol for in-memory storage
        connectionUrl = "mem://";
      }

      const startTime = performance.now();
      await instance.connect(connectionUrl);
      const endTime = performance.now();

      const selectedNamespace = namespace || "main";
      const selectedDatabase = database || dbName;
      await instance.use({
        namespace: selectedNamespace,
        database: selectedDatabase,
      });

      return {
        instance,
        connectionUrl,
        duration: (endTime - startTime).toFixed(2),
      };
    },
    catch: (error: any) => {
      return new LocalDatabaseError({
        message: "Failed to create local database",
        cause: error,
      });
    },
  });

  yield* Effect.logInfo(
    `[DatabaseService] ✅ Connected successfully! (${local.duration}ms)`
  );
  yield* Effect.logInfo(
    `[DatabaseService] ✅ Local database fully initialized! (${
      namespace || "main"
    }/${database || dbName})`
  );

  return local.instance;
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

      internalDatabase = yield* createLocalDatabase(
        localDbName,
        storageStrategy,
        "internal",
        "main"
      );

      localDatabase = yield* createLocalDatabase(
        localDbName,
        storageStrategy,
        namespace,
        database
      );

      remoteDatabase = yield* connectRemoteDatabase(remoteUrl);

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
