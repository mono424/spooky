import { Effect } from "effect";
import type { Surreal } from "surrealdb";
import { DatabaseService, makeConfig } from "./services/index.js";
import { SchemaStructure } from "@spooky/query-builder";
import { logDebug, logError, logInfo } from "./services/logger.js";

/**
 * Options for database provisioning
 */
export interface ProvisionOptions {
  /** Force re-provision even if schema already exists */
  force?: boolean;
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
export const sha1 = async (str: string): Promise<string> => {
  const enc = new TextEncoder();
  const hash = await crypto.subtle.digest("SHA-1", enc.encode(str));
  return Array.from(new Uint8Array(hash))
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("");
};

/**
 * Initializes the internal database with __schema table
 */
export const initializeInternalDatabase = (
  internalDb: Surreal
): Promise<void> => {
  return Effect.runPromise(
    logDebug("Initializing internal database...").pipe(
      Effect.andThen(() =>
        Effect.tryPromise({
          try: () =>
            internalDb
              .query(
                `
      DEFINE TABLE IF NOT EXISTS __schema SCHEMAFULL;
      DEFINE FIELD IF NOT EXISTS id ON __schema TYPE string;
      DEFINE FIELD IF NOT EXISTS hash ON __schema TYPE string;
      DEFINE FIELD IF NOT EXISTS created_at ON __schema TYPE datetime VALUE time::now();
      DEFINE INDEX IF NOT EXISTS unique_hash ON __schema FIELDS hash UNIQUE;
    `
              )
              .dispatch(),
          catch: (error) =>
            Effect.gen(function* () {
              yield* logError("Failed to initialize internal database", error);
              throw error;
            }),
        })
      )
    )
  );
};

/**
 * Checks if the current schema hash matches the stored hash
 */
export const isSchemaUpToDate = async (
  internalDb: Surreal,
  hash: string
): Promise<boolean> => {
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
};

/**
 * Drops the main database and recreates it
 */
export const dropMainDatabase = async (
  localDb: Surreal,
  database: string
): Promise<void> => {
  console.log("Dropping main database...");
  try {
    await localDb.query(`REMOVE DATABASE ${database};`).dispatch();
  } catch (error) {
    // Ignore error if database doesn't exist
  }
  await localDb.query(`DEFINE DATABASE ${database};`).dispatch();
  console.log("Main database dropped successfully");
};

/**
 * Provisions the schema by executing all SurrealQL statements
 */
export const provisionSchema = async (
  localDb: Surreal,
  schemaContent: string
): Promise<void> => {
  console.log("Provisioning new schema...");

  // Split into statements and execute them individually
  const statements = schemaContent
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  for (const statement of statements) {
    try {
      await localDb.query(statement).dispatch();
      console.info(`Executed statement:\n${statement}`);
    } catch (err: any) {
      console.error(`Error executing statement: ${statement}`);
      throw err;
    }
  }

  console.log("Schema provisioned successfully");
};

/**
 * Records the schema hash in the internal database
 */
export const recordSchemaHash = async (
  internalDb: Surreal,
  hash: string
): Promise<void> => {
  await internalDb.query(
    `UPSERT __schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
    { hash }
  ).dispatch();
  console.log("Schema hash recorded in internal database");
};

/**
 * Main provision function that orchestrates the provisioning process
 * This is the primary entry point for database schema provisioning
 */
export const provision = <S extends SchemaStructure>(
  options: ProvisionOptions = {}
) => {
  return Effect.gen(function* () {
    const { database, schemaSurql } = yield* (yield* makeConfig<S>()).getConfig;

    const databaseService = yield* DatabaseService;
    const { force = false } = options;

    const result = yield* databaseService.useInternal(async (db: Surreal) => {
      const schemaHash = await sha1(schemaSurql);
      const isUpToDate = await isSchemaUpToDate(db, schemaHash);
      let shouldMigrate = force || !isUpToDate;
      
      console.log(`[Provisioning] Schema hash: ${schemaHash}`);
      console.log(`[Provisioning] Schema up to date: ${isUpToDate}`);
      console.log(`[Provisioning] Should migrate: ${shouldMigrate}`);
      
      if (!shouldMigrate) return { shouldMigrate, schemaHash };

      console.log("[Provisioning] Initializing internal database schema...");
      await initializeInternalDatabase(db);
      shouldMigrate = true;
      return { shouldMigrate, schemaHash };
    });

    if (!result.shouldMigrate) {
      console.log("[Provisioning] Schema is up to date, skipping migration");
      return;
    }

    console.log("[Provisioning] Starting schema migration...");
    yield* databaseService.useLocal(async (db: Surreal) => {
      await dropMainDatabase(db, database);
      await provisionSchema(db, schemaSurql);
      return true;
    });

    yield* databaseService.useInternal(async (db: Surreal) => {
      await recordSchemaHash(db, result.schemaHash);
    });

    console.log("[Provisioning] Database schema provisioned successfully");
  }).pipe(
    Effect.catchAll((error) => {
      console.error("[Provisioning] Failed to provision database schema:", error);
      return Effect.fail(error);
    })
  );
};
