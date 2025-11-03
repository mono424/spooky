import { RecordId, Surreal, Uuid } from "surrealdb";
import { createNodeEngines } from "@surrealdb/node";
import { Data, Effect, Layer } from "effect";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteAuthenticationError,
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
const useMockNode = Effect.fn("useMockNode")(function* (instance: Surreal) {
  return Effect.fn("useMockNodeInner")(function* <T>(
    fn: (db: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
  ) {
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
    }
    return result;
  });
});

/**
 * Wrapper function for using local database
 */
const useLocalDatabase = Effect.fn("useLocalDatabase")(function* (db: Surreal) {
  return Effect.fn("useLocalDatabaseInner")(function* <T>(
    fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
  ) {
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

/**
 * Query function for local database
 */
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

/**
 * Query function for remote/mock database
 */
const queryMockDatabase = Effect.fn("queryMockDatabase")(function* (
  instance: Surreal
) {
  return Effect.fn("queryMockDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    return yield* Effect.tryPromise({
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
  });
});

/**
 * liveOf function for remote/mock database
 */
const liveOfMockDatabase = Effect.fn("liveOfMockDatabase")(function* (
  instance: Surreal
) {
  return Effect.fn("liveOfMockDatabase")(function* (liveUuid: Uuid) {
    return yield* Effect.tryPromise({
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
  });
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

const authenticateRemoteDatabase = Effect.fn("authenticateRemoteDatabase")(
  function* (db: Surreal) {
    return Effect.fn("authenticateRemoteDatabaseInner")(function* (
      token: string
    ) {
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
  }
);

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
        useLocal: yield* useLocalDatabase(localDatabase),
        useInternal: yield* useLocalDatabase(internalDatabase),
        useRemote: yield* useMockNode(mockRemoteDatabase),
        queryLocal: yield* queryLocalDatabase(localDatabase),
        queryInternal: yield* queryLocalDatabase(internalDatabase),
        queryRemote: yield* queryMockDatabase(mockRemoteDatabase),
        liveOfRemote: yield* liveOfMockDatabase(mockRemoteDatabase),
        authenticate: yield* authenticateRemoteDatabase(mockRemoteDatabase),
      });
    })
  );
