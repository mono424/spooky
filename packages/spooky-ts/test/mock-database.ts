import { Surreal, Uuid } from "surrealdb";
import { createNodeEngines } from "@surrealdb/node";
import { Data, Effect, Layer } from "effect";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteDatabaseError,
} from "../src/services/database.js";
import { makeConfig } from "../src/services/config.js";
import { SchemaStructure } from "@spooky/query-builder";

/**
 * Mock database implementation that uses 3 local SurrealDB WASM nodes
 * instead of a remote database. This simulates a distributed system
 * for testing purposes.
 */

export class MockNodeError extends Data.TaggedError("MockNodeError")<{
  readonly cause?: unknown;
  readonly message: string;
  readonly nodeId: string;
}> {}

/**
 * Creates a single mock SurrealDB node using WASM
 */
const createMockNode = (nodeId: string, namespace: string, database: string) =>
  Effect.tryPromise({
    try: async () => {
      dbContext.mockRemoteDatabase = await createNewDatabase(
        namespace || "main",
        database || database
      );
      return dbContext.mockRemoteDatabase;
    },
    catch: (error) =>
      new RemoteDatabaseError({
        message: `Failed to create mock remote node ${nodeId}`,
        cause: error,
      }),
  });

/**
 * Wrapper function for using a mock node
 */
const useMockNode =
  (instance: Surreal) =>
  <T>(
    fn: (db: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
  ): Effect.Effect<T, RemoteDatabaseError, never> =>
    Effect.gen(function* () {
      const result = yield* Effect.try({
        try: () => fn(instance),
        catch: (error) =>
          new RemoteDatabaseError({
            message: `Failed to use mock db [sync]`,
            cause: error,
          }),
      });

      if (result instanceof Promise) {
        return yield* Effect.tryPromise({
          try: () => result,
          catch: (error) =>
            new RemoteDatabaseError({
              message: `Failed to use mock db [async]`,
              cause: error,
            }),
        });
      } else {
        return result;
      }
    });

/**
 * Wrapper function for using local database
 */
const useLocalDatabase =
  (db: Surreal) =>
  <T>(
    fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
  ): Effect.Effect<T, LocalDatabaseError, never> =>
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

/**
 * Query function for local database
 */
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

/**
 * Query function for remote/mock database
 */
const queryMockDatabase =
  (instance: Surreal) =>
  <T>(sql: string, vars?: Record<string, unknown>) =>
    Effect.tryPromise({
      try: async () => {
        const result = await instance.query(sql, vars).collect<[T]>();
        return result[0];
      },
      catch: (error) =>
        new RemoteDatabaseError({
          message: "Failed to execute query on mock database",
          cause: error,
        }),
    });

/**
 * liveOf function for remote/mock database
 */
const liveOfMockDatabase = (instance: Surreal) => (liveUuid: Uuid) =>
  Effect.tryPromise({
    try: async () => {
      const result = await instance.liveOf(liveUuid);
      return result;
    },
    catch: (error) =>
      new RemoteDatabaseError({
        message: "Failed to execute liveOf on mock database",
        cause: error,
      }),
  });

const createNewDatabase = async (namespace: string, database: string) => {
  const db = new Surreal({
    engines: createNodeEngines({
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
  await db.connect("mem://");
  await db.use({
    namespace,
    database,
  });
  return db;
};

export const dbContext: {
  internalDatabase: Surreal | undefined;
  localDatabase: Surreal | undefined;
  mockRemoteDatabase: Surreal | undefined;
} = {
  internalDatabase: undefined,
  localDatabase: undefined,
  mockRemoteDatabase: undefined,
};

/**
 * Mock Database Service Layer that creates 3 local nodes instead of
 * connecting to a remote database
 */
export const MockDatabaseServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    DatabaseService,
    Effect.gen(function* () {
      const config = yield* (yield* makeConfig<S>()).getConfig;
      const { localDbName, namespace, database } = config;

      // Create local and internal databases (same as real implementation)
      const internalDatabase = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: async () => {
            dbContext.internalDatabase = await createNewDatabase(
              "internal",
              "main"
            );
            return dbContext.internalDatabase;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to create internal database",
              cause: error,
            }),
        }),
        (db) => Effect.promise(() => db.close())
      );

      const localDatabase = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: async () => {
            dbContext.localDatabase = await createNewDatabase(
              namespace || "main",
              database || localDbName
            );
            return dbContext.localDatabase;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to create local database",
              cause: error,
            }),
        }),
        (db) => Effect.promise(() => db.close())
      );

      const mockRemoteDatabase = yield* createMockNode(
        "remote",
        namespace || "main",
        database || "test"
      );

      return DatabaseService.of({
        useLocal: useLocalDatabase(localDatabase),
        useInternal: useLocalDatabase(internalDatabase),
        useRemote: useMockNode(mockRemoteDatabase),
        queryLocal: queryLocalDatabase(localDatabase),
        queryInternal: queryLocalDatabase(internalDatabase),
        queryRemote: queryMockDatabase(mockRemoteDatabase),
        liveOfRemote: liveOfMockDatabase(mockRemoteDatabase),
      });
    })
  );
