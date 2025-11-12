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
        console.log(`[DatabaseService] Connecting to remote database...`);
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
      console.log(`[DatabaseService] Creating WASM Surreal instance...`);

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

      console.log(
        `[DatabaseService] WASM Surreal instance created successfully`
      );

      // Determine connection URL based on storage strategy
      let connectionUrl: string;
      if (strategy === "indexeddb") {
        // Using indxdb:// protocol for IndexedDB storage
        connectionUrl = `indxdb://${dbName}`;
      } else {
        // Using mem:// protocol for in-memory storage
        connectionUrl = "mem://";
      }

      console.log(`[DatabaseService] Storage strategy: ${strategy}`);
      console.log(`[DatabaseService] Connection URL: ${connectionUrl}`);
      console.log(`[DatabaseService] DB Name: ${dbName}`);

      try {
        console.log(`[DatabaseService] Attempting to connect to database...`);
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
        console.error(`[DatabaseService] Full error:`, connectError);
        throw new Error(
          `Failed to connect to ${connectionUrl}: ${
            connectError?.message || connectError
          }`
        );
      }

      console.log(
        `[DatabaseService] Selecting namespace '${
          namespace || "main"
        }' and database '${database || dbName}'...`
      );
      await local.use({
        namespace: namespace || "main",
        database: database || dbName,
      });

      console.log(`[DatabaseService] ✅ Local database fully initialized!`);
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
