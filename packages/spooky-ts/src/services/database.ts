import { LiveSubscription, RecordId, Surreal, Uuid } from "surrealdb";
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
    useLocal: <T>(
      fn: (local: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    useInternal: <T>(
      fn: (internal: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    useRemote: <T>(
      fn: (remote: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
    ) => Effect.Effect<T, RemoteDatabaseError, never>;
    queryLocal: <T>(
      sql: string,
      vars?: Record<string, unknown>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    queryInternal: <T>(
      sql: string,
      vars?: Record<string, unknown>
    ) => Effect.Effect<T, LocalDatabaseError, never>;
    queryRemote: <T>(
      sql: string,
      vars?: Record<string, unknown>
    ) => Effect.Effect<T, RemoteDatabaseError, never>;
    liveOfRemote: (
      liveUuid: Uuid
    ) => Effect.Effect<LiveSubscription, RemoteDatabaseError, never>;
    authenticate: (
      token: string
    ) => Effect.Effect<RecordId, RemoteAuthenticationError, never>;
    deauthenticate: () => Effect.Effect<void, RemoteDatabaseError, never>;
    closeRemote: () => Effect.Effect<void, RemoteDatabaseError, never>;
    closeLocal: () => Effect.Effect<void, LocalDatabaseError, never>;
    closeInternal: () => Effect.Effect<void, LocalDatabaseError, never>;
    clearLocalCache: () => Effect.Effect<void, LocalDatabaseError, never>;
  }
>() {}
