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
 * Wrapper function for using a mock node
 */
const useMockNode = Effect.fn("useMockNode")(function* (
  instanceEffect: Effect.Effect<Surreal, RemoteDatabaseError, never>
) {
  return Effect.fn("useMockNodeInner")(function* <T>(
    fn: (db: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
  ) {
    const instance = yield* instanceEffect;
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
const useLocalDatabase = Effect.fn("useLocalDatabase")(function* (
  dbEffect: Effect.Effect<Surreal, LocalDatabaseError, never>
) {
  return Effect.fn("useLocalDatabaseInner")(function* <T>(
    fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
  ) {
    const db = yield* dbEffect;
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
  dbEffect: Effect.Effect<Surreal, LocalDatabaseError, never>
) {
  return Effect.fn("queryLocalDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    const db = yield* dbEffect;
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

const clearDatabase = Effect.fn("clearLocalCache")(function* (
  dbEffect: Effect.Effect<Surreal, LocalDatabaseError, never>,
  tables: SchemaStructure["tables"]
) {
  return Effect.fn("clearDatabaseInner")(function* () {
    const db = yield* dbEffect;
    return yield* Effect.tryPromise({
      try: async () => {
        for (const table of tables) {
          await db.query(`DELETE ${table.name}`).collect();
        }
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
  instanceEffect: Effect.Effect<Surreal, RemoteDatabaseError, never>
) {
  return Effect.fn("queryMockDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    const instance = yield* instanceEffect;

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
  instanceEffect: Effect.Effect<Surreal, RemoteDatabaseError, never>
) {
  return Effect.fn("liveOfMockDatabase")(function* (liveUuid: Uuid) {
    const instance = yield* instanceEffect;
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
    engines: createNodeEngines({}),
    codecOptions: {
      useNativeDates: false,
    },
  });
  await db.connect(
    "mem://" + database + Math.random().toString(36).substring(2, 15)
  );
  await db.use({
    namespace,
    database,
  });

  db.subscribe("disconnected", () => console.log("[SURREALDB] DISCONNECTED"));
  db.subscribe("connected", () => console.log("[SURREALDB] CONNECTED"));
  db.subscribe("reconnecting", () => console.log("[SURREALDB] RECONNECTING"));
  db.subscribe("error", (err) => console.log("[SURREALDB] ERROR | ", err));
  return db;
};

export const dbContext: {
  internalDatabase: Surreal | undefined;
  localDatabase: Surreal | undefined;
  remoteDatabase: Surreal | undefined;
} = {
  internalDatabase: undefined,
  localDatabase: undefined,
  remoteDatabase: undefined,
};

const authenticateRemoteDatabase = Effect.fn("authenticateRemoteDatabase")(
  function* (dbEffect: Effect.Effect<Surreal, RemoteDatabaseError, never>) {
    return Effect.fn("authenticateRemoteDatabaseInner")(function* (
      token: string
    ) {
      const db = yield* dbEffect.pipe(
        Effect.mapError(
          (error) =>
            new RemoteAuthenticationError({
              message: "Failed to use mock database",
              cause: error,
            })
        )
      );
      return yield* Effect.tryPromise({
        try: async () => {
          await db.authenticate(token);
          const [result] = await db
            .query(`SELECT id FROM $auth`)
            .collect<[[{ id: RecordId }]]>();
          if (!result[0]) {
            throw new RemoteAuthenticationError({
              message: "Failed to authenticate on remote database",
              cause: new Error("User not found"),
            });
          }
          return result[0]?.id;
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
      const {
        localDbName,
        namespace,
        database,
        schema: { tables },
      } = config;

      // Create local and internal databases (same as real implementation)
      const internalDatabase = Effect.tryPromise({
        try: async () => {
          if (dbContext.internalDatabase) return dbContext.internalDatabase;
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
      });

      const localDatabase = Effect.tryPromise({
        try: async () => {
          if (dbContext.localDatabase) return dbContext.localDatabase;
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
      });

      const remoteDatabase = Effect.tryPromise({
        try: async () => {
          if (dbContext.remoteDatabase) return dbContext.remoteDatabase;
          dbContext.remoteDatabase = await createNewDatabase(
            namespace || "main",
            database || localDbName
          );
          return dbContext.remoteDatabase;
        },
        catch: (error) =>
          new RemoteDatabaseError({
            message: "Failed to create remote database",
            cause: error,
          }),
      });

      const closeRemoteDb = Effect.fn("closeRemoteDatabase")(function* () {
        return yield* Effect.tryPromise({
          try: async () => {
            await dbContext.remoteDatabase?.close();
            dbContext.remoteDatabase = undefined;
          },
          catch: (error) =>
            new RemoteDatabaseError({
              message: "Failed to close remote database",
              cause: error,
            }),
        });
      });

      const closeLocalDb = Effect.fn("closeLocalDatabase")(function* () {
        return yield* Effect.tryPromise({
          try: async () => {
            await dbContext.localDatabase?.close();
            dbContext.localDatabase = undefined;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to close local database",
              cause: error,
            }),
        });
      });

      const closeInternalDb = Effect.fn("closeInternalDatabase")(function* () {
        return yield* Effect.tryPromise({
          try: async () => {
            await dbContext.internalDatabase?.close();
            dbContext.internalDatabase = undefined;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to close local database",
              cause: error,
            }),
        });
      });

      // Important to make dbContext work for the tests
      yield* Effect.log("remoteDatabase", yield* remoteDatabase);
      yield* Effect.log("localDatabase", yield* localDatabase);
      yield* Effect.log("internalDatabase", yield* internalDatabase);
      // Important to make dbContext work for the tests

      return DatabaseService.of({
        useLocal: yield* useLocalDatabase(localDatabase),
        useInternal: yield* useLocalDatabase(internalDatabase),
        useRemote: yield* useMockNode(remoteDatabase),
        queryLocal: yield* queryLocalDatabase(localDatabase),
        queryInternal: yield* queryLocalDatabase(internalDatabase),
        queryRemote: yield* queryMockDatabase(remoteDatabase),
        liveOfRemote: yield* liveOfMockDatabase(remoteDatabase),
        authenticate: yield* authenticateRemoteDatabase(remoteDatabase),
        closeRemote: closeRemoteDb,
        closeLocal: closeLocalDb,
        closeInternal: closeInternalDb,
        clearLocalCache: yield* clearDatabase(localDatabase, tables),
      });
    })
  );
