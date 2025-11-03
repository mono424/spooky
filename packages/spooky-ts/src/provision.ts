import { Effect } from "effect";
import type { Surreal } from "surrealdb";
import {
  DatabaseService,
  LocalDatabaseError,
  makeConfig,
  RemoteDatabaseError,
} from "./services/index.js";
import { SchemaStructure } from "@spooky/query-builder";

/**
 * Options for database provisioning
 */
export interface ProvisionOptions {
  /** Force re-provision even if schema already exists */
  force?: boolean;
  /** Provision the remote database */
  provisionRemote?: boolean;
}

/**
 * Schema record stored in internal database
 */
export interface SchemaRecord {
  hash: string;
  created_at: string;
}

/**
 * Context required for provisioning operations
 */
export interface ProvisionContext {
  internalDb: Surreal;
  localDb: Surreal;
  namespace: string;
  database: string;
  internalDatabase: string;
  schema: string;
}

/**
 * Computes SHA-1 hash of a string using Web Crypto API
 */
export const sha1 = Effect.fn("sha1")(function* (str: string) {
  return yield* Effect.tryPromise({
    try: async () => {
      const enc = new TextEncoder();
      const hash = await crypto.subtle.digest("SHA-1", enc.encode(str));
      return Array.from(new Uint8Array(hash))
        .map((v) => v.toString(16).padStart(2, "0"))
        .join("");
    },
    catch: (error) => new Error(`Failed to compute SHA-1 hash: ${error}`),
  });
});

/**
 * Initializes the internal database with __schema table
 */
export const initializeInternalDatabase = Effect.fn(
  "initializeInternalDatabase"
)(function* (internalDb: Surreal) {
  return yield* Effect.tryPromise({
    try: async () => {
      console.log("Initializing internal database...", internalDb);
      await internalDb
        .query(
          `
          DEFINE TABLE IF NOT EXISTS __schema SCHEMAFULL;
          DEFINE FIELD IF NOT EXISTS id ON __schema TYPE string;
          DEFINE FIELD IF NOT EXISTS hash ON __schema TYPE string;
          DEFINE FIELD IF NOT EXISTS created_at ON __schema TYPE datetime VALUE time::now();
          DEFINE INDEX IF NOT EXISTS unique_hash ON __schema FIELDS hash UNIQUE;
        `
        )
        .dispatch();
      return Effect.succeed(undefined);
    },
    catch: (error) =>
      new Error(`Failed to initialize internal database: ${error}`),
  });
});

/**
 * Checks if the current schema hash matches the stored hash
 */
export const isSchemaUpToDate = Effect.fn("isSchemaUpToDate")(function* (
  internalDb: Surreal,
  hash: string
) {
  return yield* Effect.tryPromise({
    try: async () => {
      try {
        const [result] = await internalDb
          .query(
            `SELECT hash, created_at FROM __schema ORDER BY created_at DESC LIMIT 1;`
          )
          .collect<[SchemaRecord[]]>();

        if (result.length > 0) {
          return result[0].hash === hash;
        }
        return false;
      } catch (error) {
        console.error("Error checking schema up to date:", error);
        console.log("Internal database not initialized yet");
        return false;
      }
    },
    catch: (error) => new Error(`Failed to check schema status: ${error}`),
  });
});

/**
 * Drops the main database and recreates it
 */
export const dropMainDatabase = Effect.fn("dropMainDatabase")(function* (
  localDb: Surreal,
  database: string
) {
  Effect.tryPromise({
    try: async () => {
      console.log("Dropping main database...");
      try {
        await localDb.query(`REMOVE DATABASE ${database};`);
      } catch (error) {
        // Ignore error if database doesn't exist
      }
      await localDb.query(`DEFINE DATABASE ${database};`);
      console.log("Main database dropped successfully");
      return Effect.succeed(undefined);
    },
    catch: (error) => new Error(`Failed to drop main database: ${error}`),
  });
});

/**
 * Provisions the schema by executing all SurrealQL statements
 */
export const provisionSchema = Effect.fn("provisionSchema")(function* (
  localDb: Surreal,
  schemaContent: string
) {
  return yield* Effect.tryPromise({
    try: async () => {
      console.log("Provisioning new schema...");

      // Split into statements and execute them individually
      const statements = schemaContent
        .split(";")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);

      for (const statement of statements) {
        try {
          await localDb.query(statement);
          console.info(`Executed statement:\n${statement}`);
        } catch (err: any) {
          console.error(`Error executing statement: ${statement}`);
          throw err;
        }
      }

      console.log("Schema provisioned successfully");
      return Effect.succeed(undefined);
    },
    catch: (error) => new Error(`Failed to provision schema: ${error}`),
  });
});

/**
 * Records the schema hash in the internal database
 */
export const recordSchemaHash = Effect.fn("recordSchemaHash")(function* (
  internalDb: Surreal,
  hash: string
) {
  return yield* Effect.tryPromise({
    try: async () => {
      await internalDb.query(
        `UPSERT __schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
        { hash }
      );
      console.log("Schema hash recorded in internal database");
      return Effect.succeed(undefined);
    },
    catch: (error) => new Error(`Failed to record schema hash: ${error}`),
  });
});

/**
 * Main provision function that orchestrates the provisioning process
 * This is the primary entry point for database schema provisioning
 */
export const provision = Effect.fn("provision")(function* <
  S extends SchemaStructure
>(options: ProvisionOptions = {}) {
  return yield* Effect.gen(function* () {
    const { database, schemaSurql } = yield* (yield* makeConfig<S>()).getConfig;

    const databaseService = yield* DatabaseService;
    const { force = false, provisionRemote = false } = options;

    yield* Effect.gen(function* () {
      const result = yield* databaseService.useInternal(
        Effect.fn("shouldMigrate")(function* (db) {
          return yield* Effect.gen(function* () {
            const schemaHash = yield* sha1(schemaSurql);
            const isUpToDate = yield* isSchemaUpToDate(db, schemaHash);
            const shouldMigrate = force || !isUpToDate;
            if (!shouldMigrate)
              return Effect.succeed({ shouldMigrate, schemaHash });

            yield* initializeInternalDatabase(db);
            return Effect.succeed({ shouldMigrate, schemaHash });
          }).pipe(
            Effect.catchAll((error) => {
              return Effect.fail(
                new LocalDatabaseError({
                  message: `Failed to use internal database: ${error}`,
                  cause: error,
                })
              );
            })
          );
        })
      );

      if (!(yield* result).shouldMigrate) return;

      yield* databaseService.useLocal(
        Effect.fn("useLocalMigration")(function* (db) {
          return yield* Effect.gen(function* () {
            yield* dropMainDatabase(db, database);
            yield* provisionSchema(db, schemaSurql);
            return true;
          }).pipe(
            Effect.catchAll((error) => {
              return Effect.fail(
                new LocalDatabaseError({
                  message: `Failed to migrate database: ${error}`,
                  cause: error,
                })
              );
            })
          );
        })
      );

      if (provisionRemote) {
        yield* databaseService.useRemote(
          Effect.fn("migrateRemote")(function* (db: Surreal) {
            return yield* Effect.gen(function* () {
              yield* provisionSchema(db, schemaSurql);
            }).pipe(
              Effect.catchAll((error) => {
                return Effect.fail(
                  new RemoteDatabaseError({
                    message: `Failed to migrate remote database: ${error}`,
                    cause: error,
                  })
                );
              })
            );
          })
        );
      }

      yield* databaseService.useInternal(
        Effect.fn("shouldMigrate")(function* (db) {
          return yield* Effect.gen(function* () {
            yield* recordSchemaHash(db, (yield* result).schemaHash);
          }).pipe(
            Effect.catchAll((error) => {
              return Effect.fail(
                new LocalDatabaseError({
                  message: `Failed to use internal database: ${error}`,
                  cause: error,
                })
              );
            })
          );
        })
      );
    });

    console.log("Database schema provisioned successfully");
    return Effect.succeed(undefined);
  }).pipe(
    Effect.catchAll((error) => {
      console.error("Failed to provision database schema:", error);
      return Effect.fail(error);
    })
  );
});
