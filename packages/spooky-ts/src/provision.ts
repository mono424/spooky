import { Effect } from "effect";
import type { Surreal } from "surrealdb";
import { DatabaseService, makeConfig } from "./services/index.js";
import { SchemaStructure } from "@spooky/query-builder";

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
export const initializeInternalDatabase = async (
  internalDb: Surreal
): Promise<void> => {
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
  try {
    await localDb.query(`REMOVE DATABASE ${database};`).dispatch();
  } catch (error) {
    // Ignore error if database doesn't exist
  }
  await localDb.query(`DEFINE DATABASE ${database};`).dispatch();
};

/**
 * Provisions the schema by executing all SurrealQL statements
 */
export const provisionSchema = async (
  localDb: Surreal,
  schemaContent: string
): Promise<void> => {
  // Split into statements and execute them individually
  const statements = schemaContent
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  for (const statement of statements) {
    await localDb.query(statement).dispatch();
  }
};

/**
 * Records the schema hash in the internal database
 */
export const recordSchemaHash = async (
  internalDb: Surreal,
  hash: string
): Promise<void> => {
  await internalDb
    .query(
      `UPSERT __schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
      { hash }
    )
    .dispatch();
};

export const provisionProgram = <S extends SchemaStructure>(
  options: ProvisionOptions = {}
) =>
  Effect.gen(function* () {
    const { database, schemaSurql } = yield* (yield* makeConfig<S>()).getConfig;

    const databaseService = yield* DatabaseService;
    const { force = false } = options;

    yield* Effect.logInfo("[Provisioning] Starting provision check...");

    const result = yield* databaseService.useInternal(async (db: Surreal) => {
      const schemaHash = await sha1(schemaSurql);
      const isUpToDate = await isSchemaUpToDate(db, schemaHash);
      let shouldMigrate = force || !isUpToDate;

      return { shouldMigrate, schemaHash, isUpToDate };
    });

    yield* Effect.logDebug(`[Provisioning] Schema hash: ${result.schemaHash}`);
    yield* Effect.logDebug(
      `[Provisioning] Schema up to date: ${result.isUpToDate}`
    );
    yield* Effect.logDebug(
      `[Provisioning] Should migrate: ${result.shouldMigrate}`
    );

    if (!result.shouldMigrate) {
      yield* Effect.logInfo(
        "[Provisioning] Schema is up to date, skipping migration"
      );
      return;
    }

    yield* Effect.logInfo(
      "[Provisioning] Initializing internal database schema..."
    );
    yield* databaseService.useInternal(async (db: Surreal) => {
      await initializeInternalDatabase(db);
    });

    yield* Effect.logInfo("[Provisioning] Starting schema migration...");
    yield* databaseService.useLocal(async (db: Surreal) => {
      await dropMainDatabase(db, database);
      await provisionSchema(db, schemaSurql);
    });

    yield* Effect.logDebug("[Provisioning] Recording schema hash...");
    yield* databaseService.useInternal(async (db: Surreal) => {
      await recordSchemaHash(db, result.schemaHash);
    });

    yield* Effect.logInfo(
      "[Provisioning] Database schema provisioned successfully"
    );
  });
