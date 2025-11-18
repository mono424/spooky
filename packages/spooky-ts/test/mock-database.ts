import { RecordId, Surreal, Uuid } from "surrealdb";
import { createNodeEngines } from "@surrealdb/node";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteAuthenticationError,
  RemoteDatabaseError,
} from "../src/services/database.js";
import { SchemaStructure } from "@spooky/query-builder";
import { SpookyConfig } from "../src/services/config.js";
import { Logger } from "../src/services/logger.js";
import {
  makeAuthenticateRemoteDatabase,
  makeCloseLocalDatabase,
  makeCloseRemoteDatabase,
  makeLiveOfRemoteDatabase,
  makeQueryLocalDatabase,
  makeQueryRemoteDatabase,
  makeUseLocalDatabase,
  makeUseRemoteDatabase,
} from "../src/services/database.js";

/**
 * Mock database implementation that uses 3 local SurrealDB nodes
 * instead of a remote database. This simulates a distributed system
 * for testing purposes.
 */

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

const makeClearDatabase = (db: Surreal, tables: SchemaStructure["tables"]) => {
  return async (): Promise<void> => {
    try {
      for (const table of tables) {
        await db.query(`DELETE ${table.name}`).collect();
      }
    } catch (error) {
      throw new LocalDatabaseError(
        "Failed to clear database",
        error
      );
    }
  };
};

/**
 * Mock Database Service that creates 3 local nodes instead of
 * connecting to a remote database
 */
export async function createMockDatabaseService<S extends SchemaStructure>(
  config: SpookyConfig<S>,
  logger: Logger
): Promise<DatabaseService> {
  const {
    localDbName,
    namespace,
    database,
    schema: { tables },
  } = config;

  // Create local and internal databases (same as real implementation)
  if (!dbContext.internalDatabase) {
    dbContext.internalDatabase = await createNewDatabase("internal", "main");
  }

  if (!dbContext.localDatabase) {
    dbContext.localDatabase = await createNewDatabase(
      namespace || "main",
      database || localDbName
    );
  }

  if (!dbContext.remoteDatabase) {
    dbContext.remoteDatabase = await createNewDatabase(
      namespace || "main",
      database || localDbName
    );
  }

  const closeRemoteDb = async (): Promise<void> => {
    try {
      await dbContext.remoteDatabase?.close();
      dbContext.remoteDatabase = undefined;
    } catch (error) {
      throw new RemoteDatabaseError("Failed to close remote database", error);
    }
  };

  const closeLocalDb = async (): Promise<void> => {
    try {
      await dbContext.localDatabase?.close();
      dbContext.localDatabase = undefined;
    } catch (error) {
      throw new LocalDatabaseError("Failed to close local database", error);
    }
  };

  const closeInternalDb = async (): Promise<void> => {
    try {
      await dbContext.internalDatabase?.close();
      dbContext.internalDatabase = undefined;
    } catch (error) {
      throw new LocalDatabaseError("Failed to close internal database", error);
    }
  };

  return {
    useLocal: makeUseLocalDatabase(dbContext.localDatabase),
    useInternal: makeUseLocalDatabase(dbContext.internalDatabase),
    useRemote: makeUseRemoteDatabase(dbContext.remoteDatabase),
    queryLocal: makeQueryLocalDatabase(dbContext.localDatabase, logger),
    queryInternal: makeQueryLocalDatabase(dbContext.internalDatabase, logger),
    queryRemote: makeQueryRemoteDatabase(dbContext.remoteDatabase, logger),
    liveOfRemote: makeLiveOfRemoteDatabase(dbContext.remoteDatabase),
    authenticate: makeAuthenticateRemoteDatabase(dbContext.remoteDatabase),
    deauthenticate: async () => {
      await dbContext.remoteDatabase?.invalidate();
    },
    closeRemote: closeRemoteDb,
    closeLocal: closeLocalDb,
    closeInternal: closeInternalDb,
    clearLocalCache: makeClearDatabase(dbContext.localDatabase, tables),
  };
}
