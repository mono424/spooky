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
    yield* Effect.logDebug(`[DatabaseService] Connecting to remote database...`);
    return yield* Effect.tryPromise({
      try: async () => {
        const remote = new Surreal({
          engines: createRemoteEngines(),
          codecOptions: { useNativeDates: false },
        });
        const startTime = performance.now();
        await remote.connect(url);
        const endTime = performance.now();
        console.log(
          `[DatabaseService] ✅ Connected successfully! (${(
            endTime - startTime
          ).toFixed(2)}ms)`
        );
        console.log(`[DatabaseService] URL: ${url}`);
        return remote;
      },
      catch: (error) => {
        console.error(
          `[DatabaseService] Error connecting to remote database: ${error}`
        );
        return new RemoteDatabaseError({
          message: "Failed to connect to remote database",
          cause: error,
        });
      },
    }).pipe(
      Effect.tap(() => 
        Effect.logInfo(
          `[DatabaseService] ✅ Connected successfully to remote database`
        )
      ),
      Effect.catchAll((error) =>
        Effect.gen(function* () {
          yield* Effect.logError(
            `[DatabaseService] Failed to connect to remote database`,
            error
          );
          throw error;
        })
      )
    );
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

      // Determine connection URL based on storage strategy
      let connectionUrl: string;
      if (strategy === "indexeddb") {
        // Using indxdb:// protocol for IndexedDB storage
        connectionUrl = `indxdb://${dbName}`;
      } else {
        // Using mem:// protocol for in-memory storage
        connectionUrl = "mem://";
      }

      try {
        const startTime = performance.now();
        await local.connect(connectionUrl);
        const endTime = performance.now();
        console.log(
          `[DatabaseService] ✅ Connected successfully! (${(
            endTime - startTime
          ).toFixed(2)}ms)`
        );
      } catch (connectError: any) {
        console.error(
          `[DatabaseService] ❌ Connection FAILED after attempting to connect to ${connectionUrl}`
        );
        console.error(
          `[DatabaseService] Error type:`,
          connectError?.constructor?.name || typeof connectError
        );
        console.error(
          `[DatabaseService] Error message:`,
          connectError?.message || connectError
        );
        throw new Error(
          `Failed to connect to ${connectionUrl}: ${
            connectError?.message || connectError
          }`
        );
      }

      const selectedNamespace = namespace || "main";
      const selectedDatabase = database || dbName;
      await local.use({
        namespace: selectedNamespace,
        database: selectedDatabase,
      });

      return local;
    },
    catch: (error: any) => {
      console.error(
        "[DatabaseService] ❌ Error creating local database:",
        error?.message || error
      );
      return new LocalDatabaseError({
        message: "Failed to create local database",
        cause: error,
      });
    },
  }).pipe(
    Effect.tap(() =>
      Effect.logInfo(
        `[DatabaseService] ✅ Local database fully initialized! (${namespace || "main"}/${database || dbName})`
      )
    ),
    Effect.catchAll((error) =>
      Effect.gen(function* () {
        yield* Effect.logError(
          `[DatabaseService] Failed to create local database`,
          error
        );
        throw error;
      })
    )
  );
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
