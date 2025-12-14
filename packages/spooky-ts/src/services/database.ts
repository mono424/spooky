import { LiveHandler, RecordId, Surreal, Uuid } from "surrealdb";
import { Logger } from "./logger.js";

export class LocalDatabaseError extends Error {
  readonly cause?: unknown;
  constructor(message: string, cause?: unknown) {
    super(message);
    this.name = "LocalDatabaseError";
    this.cause = cause;
  }
}

export class RemoteDatabaseError extends Error {
  readonly cause?: unknown;
  constructor(message: string, cause?: unknown) {
    super(message);
    this.name = "RemoteDatabaseError";
    this.cause = cause;
  }
}

export class RemoteAuthenticationError extends Error {
  readonly cause?: unknown;
  constructor(message: string, cause?: unknown) {
    super(message);
    this.name = "RemoteAuthenticationError";
    this.cause = cause;
  }
}

export interface DatabaseService {
  useLocal: <T>(fn: (db: Surreal) => T | Promise<T>) => Promise<T>;
  useInternal: <T>(fn: (db: Surreal) => T | Promise<T>) => Promise<T>;
  useRemote: <T>(fn: (db: Surreal) => T | Promise<T>) => Promise<T>;
  queryLocal: <T>(sql: string, vars?: Record<string, unknown>) => Promise<T>;
  queryInternal: <T>(sql: string, vars?: Record<string, unknown>) => Promise<T>;
  queryRemote: <T>(sql: string, vars?: Record<string, unknown>) => Promise<T>;
  subscribeLiveOfRemote: (
    liveUuid: Uuid,
    callback: LiveHandler<Record<string, unknown>>
  ) => Promise<any>;
  unsubscribeLiveOfRemote: (
    liveUuid: Uuid,
    callback: LiveHandler<Record<string, unknown>>
  ) => Promise<any>;
  authenticate: (token: string) => Promise<RecordId | undefined>;
  deauthenticate: () => Promise<void>;
  closeRemote: () => Promise<void>;
  closeLocal: () => Promise<void>;
  closeInternal: () => Promise<void>;
  clearLocalCache: () => Promise<void>;
}

export const makeUseLocalDatabase = (db: Surreal) => {
  return async <T>(fn: (db: Surreal) => T | Promise<T>): Promise<T> => {
    try {
      const result = fn(db);
      if (result instanceof Promise) {
        return await result;
      }
      return result;
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new LocalDatabaseError(`Failed to use database: ${msg}`, error);
    }
  };
};

export const makeUseRemoteDatabase = (db: Surreal) => {
  return async <T>(fn: (db: Surreal) => T | Promise<T>): Promise<T> => {
    try {
      const result = fn(db);
      if (result instanceof Promise) {
        return await result;
      }
      return result;
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(`Failed to use database: ${msg}`, error);
    }
  };
};

export const makeQueryRemoteDatabase = (db: Surreal, logger: Logger) => {
  return async <T>(sql: string, vars?: Record<string, unknown>): Promise<T> => {
    try {
      // In surrealdb 1.x, query returns [result] where result is an array of rows
      const result = vars
        ? await db.query<T[]>(sql, vars)
        : await db.query<T[]>(sql);
      logger.debug("[Database] Query Remote Database - Result", {
        sql,
        vars,
        result,
      });
      // Return the array of rows (result[0]), not just the first row
      // This allows callers to get arrays when they expect arrays, or destructure when needed
      if (result && result.length > 0) {
        if (Array.isArray(result[0])) {
          // result[0] is an array of rows (normal SELECT query)
          return result[0] as T;
        } else {
          // result[0] is a single value (LIVE query or single-row result)
          // Wrap it in an array so callers can destructure if needed
          return [result[0]] as unknown as T;
        }
      }
      // If no results, return empty array for array types
      return [] as unknown as T;
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(
        `Failed to execute query on remote database: ${msg}`,
        error
      );
    }
  };
};

export const makeQueryLocalDatabase = (db: Surreal, logger: Logger) => {
  return async <T>(sql: string, vars?: Record<string, unknown>): Promise<T> => {
    try {
      // In surrealdb 1.x, query returns [result] where result is an array of rows
      const result = vars
        ? await db.query<T[]>(sql, vars)
        : await db.query<T[]>(sql);
      logger.debug("[Database] Query Local Database - Result", {
        sql,
        vars,
        result,
      });
      // Return the array of rows (result[0]), not just the first row
      // This allows callers to get arrays when they expect arrays, or destructure when needed
      if (result && result.length > 0) {
        if (Array.isArray(result[0])) {
          // result[0] is an array of rows (normal SELECT query)
          return result[0] as T;
        } else {
          // result[0] is a single value (LIVE query or single-row result)
          // Wrap it in an array so callers can destructure if needed
          return [result[0]] as unknown as T;
        }
      }
      // If no results, return empty array for array types
      return [] as unknown as T;
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new LocalDatabaseError(
        `Failed to execute query on local database: ${msg}`,
        error
      );
    }
  };
};

export const makeSubscribeLiveOfRemoteDatabase = (db: Surreal) => {
  return async (
    liveUuid: Uuid,
    callback: LiveHandler<Record<string, unknown>>
  ) => {
    try {
      // In surrealdb 1.x, it's `live` not `liveOf`, and it takes a string
      return await db.subscribeLive(liveUuid, callback);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(
        `Failed to execute live on remote database: ${msg}`,
        error
      );
    }
  };
};

export const makeUnsubscribeLiveOfRemoteDatabase = (db: Surreal) => {
  return async (
    liveUuid: Uuid,
    callback: LiveHandler<Record<string, unknown>>
  ) => {
    try {
      return await db.unSubscribeLive(liveUuid, callback);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(
        `Failed to unsubscribe from live on remote database: ${msg}`,
        error
      );
    }
  };
};

export const makeAuthenticateRemoteDatabase = (db: Surreal) => {
  return async (token: string): Promise<RecordId | undefined> => {
    try {
      await db.authenticate(token);
      const result = await db.query<{ id: RecordId }[]>(`SELECT id FROM $auth`);
      // In surrealdb 1.x, query returns [result] where result is an array of rows
      if (
        result &&
        result.length > 0 &&
        Array.isArray(result[0]) &&
        result[0].length > 0
      ) {
        return result[0][0]?.id;
      }
      return undefined;
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteAuthenticationError(
        `Failed to authenticate on remote database: ${msg}`,
        error
      );
    }
  };
};

export const makeDeauthenticateRemoteDatabase = (db: Surreal) => {
  return async (): Promise<void> => {
    try {
      await db.invalidate();
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(
        `Failed to deauthenticate on remote database: ${msg}`,
        error
      );
    }
  };
};

export const makeCloseRemoteDatabase = (db: Surreal) => {
  return async (): Promise<void> => {
    try {
      await db.close();
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new RemoteDatabaseError(`Failed to close remote database: ${msg}`, error);
    }
  };
};

export const makeCloseLocalDatabase = (db: Surreal) => {
  return async (): Promise<void> => {
    try {
      await db.close();
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new LocalDatabaseError(`Failed to close local database: ${msg}`, error);
    }
  };
};

export const makeClearLocalCache = (db: Surreal) => {
  return async (): Promise<void> => {
    try {
      // Get all tables and delete all records from them
      const result = await db.query<[{ tables: Record<string, unknown> }]>(
        "INFO FOR DB"
      );
      // In surrealdb 1.x, query returns [result] where result is an array of rows
      const info =
        result &&
        result.length > 0 &&
        Array.isArray(result[0]) &&
        result[0].length > 0
          ? (result[0][0] as { tables: Record<string, unknown> })
          : null;

      if (info?.tables) {
        const tableNames = Object.keys(info.tables);
        for (const tableName of tableNames) {
          await db.query(`DELETE ${tableName}`);
        }
      }
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      throw new LocalDatabaseError(`Failed to clear local cache: ${msg}`, error);
    }
  };
};
