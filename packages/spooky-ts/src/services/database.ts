import { RecordId, Surreal, Uuid } from "surrealdb";
import { Context, Data, Effect } from "effect";

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

export class RemoteAuthenticationError extends Data.TaggedError(
  "RemoteAuthenticationError"
)<{
  readonly cause?: unknown;
  readonly message: string;
}> {}

export class DatabaseService extends Context.Tag("DatabaseService")<
  DatabaseService,
  {
    useLocal: ReturnType<typeof makeUseLocalDatabase>;
    useInternal: ReturnType<typeof makeUseLocalDatabase>;
    useRemote: ReturnType<typeof makeUseRemoteDatabase>;
    queryLocal: ReturnType<typeof makeQueryLocalDatabase>;
    queryInternal: ReturnType<typeof makeQueryLocalDatabase>;
    queryRemote: ReturnType<typeof makeQueryRemoteDatabase>;
    liveOfRemote: ReturnType<typeof makeLiveOfRemoteDatabase>;
    authenticate: ReturnType<typeof makeAuthenticateRemoteDatabase>;
    deauthenticate: ReturnType<typeof makeDeauthenticateRemoteDatabase>;
    closeRemote: ReturnType<typeof makeCloseRemoteDatabase>;
    closeLocal: ReturnType<typeof makeCloseLocalDatabase>;
    closeInternal: ReturnType<typeof makeCloseLocalDatabase>;
    clearLocalCache: ReturnType<typeof makeClearLocalCache>;
  }
>() {}

export const makeUseLocalDatabase = (db: Surreal) => {
  return Effect.fn("useLocalDatabase")(function* <T>(
    fn: (db: Surreal) => T | Promise<T>
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
};

export const makeUseRemoteDatabase = (db: Surreal) => {
  return Effect.fn("useRemoteDatabaseInner")(
    <T>(fn: (db: Surreal) => T | Promise<T>) =>
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
};

export const makeQueryRemoteDatabase = (db: Surreal) => {
  return Effect.fn("queryRemoteDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    return yield* Effect.tryPromise({
      try: async () => {
        const result = await db.query(sql, vars).collect<[T]>();
        Effect.logDebug("[Database] Query Remote Database - Result", {
          sql,
          vars,
          result,
        });
        return result[0];
      },
      catch: (error) =>
        new RemoteDatabaseError({
          message: "Failed to execute query on remote database",
          cause: error,
        }),
    });
  });
};

export const makeQueryLocalDatabase = (db: Surreal) => {
  return Effect.fn("queryLocalDatabaseInner")(function* <T>(
    sql: string,
    vars?: Record<string, unknown>
  ) {
    return yield* Effect.tryPromise({
      try: async () => {
        const result = await db.query(sql, vars).collect<[T]>();
        Effect.logDebug("[Database] Query Local Database - Result", {
          sql,
          vars,
          result,
        });
        return result[0];
      },
      catch: (error) =>
        new LocalDatabaseError({
          message: "Failed to execute query on local database",
          cause: error,
        }),
    });
  });
};

export const makeLiveOfRemoteDatabase = (db: Surreal) => {
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
};

export const makeAuthenticateRemoteDatabase = (db: Surreal) => {
  return Effect.fn("authenticateRemoteDatabase")(function* (token: string) {
    return yield* Effect.tryPromise({
      try: async () => {
        await db.authenticate(token);
        const [result] = await db
          .query(`SELECT id FROM $auth`)
          .collect<[{ id: RecordId }[]]>();
        return result?.[0]?.id;
      },
      catch: (error) =>
        new RemoteAuthenticationError({
          message: "Failed to authenticate on remote database",
          cause: error,
        }),
    });
  });
};

export const makeDeauthenticateRemoteDatabase = (db: Surreal) => {
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

export const makeCloseRemoteDatabase = (db: Surreal) => {
  return Effect.fn("deauthenticateRemoteDatabase")(function* () {
    return yield* Effect.tryPromise({
      try: async () => {
        await db.close();
      },
      catch: (error) =>
        new RemoteDatabaseError({
          message: "Failed to close remote database",
          cause: error,
        }),
    });
  });
};

export const makeCloseLocalDatabase = (db: Surreal) => {
  return Effect.fn("closeLocalDatabase")(function* () {
    return yield* Effect.tryPromise({
      try: async () => {
        await db.close();
      },
      catch: (error) =>
        new LocalDatabaseError({
          message: "Failed to close local database",
          cause: error,
        }),
    });
  });
};

export const makeClearLocalCache = (db: Surreal) => {
  return Effect.fn("clearLocalCache")(function* () {
    return yield* Effect.tryPromise({
      try: async () => {
        // Get all tables and delete all records from them
        const [info] = await db.query("INFO FOR DB").collect<
          [
            {
              tables: Record<string, unknown>;
            }
          ]
        >();

        if (info?.tables) {
          const tableNames = Object.keys(info.tables);
          for (const tableName of tableNames) {
            await db.query(`DELETE ${tableName}`).collect();
          }
        }
      },
      catch: (error) =>
        new LocalDatabaseError({
          message: "Failed to clear local cache",
          cause: error,
        }),
    });
  });
};
