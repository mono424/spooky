import type { Surreal } from "surrealdb";
import { DatabaseService } from "./services/index.js";
import { Logger } from "./services/logger.js";

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
  await internalDb.query(
    `
    DEFINE TABLE IF NOT EXISTS __schema SCHEMAFULL;
    DEFINE FIELD IF NOT EXISTS id ON __schema TYPE string;
    DEFINE FIELD IF NOT EXISTS hash ON __schema TYPE string;
    DEFINE FIELD IF NOT EXISTS created_at ON __schema TYPE datetime VALUE time::now();
    DEFINE INDEX IF NOT EXISTS unique_hash ON __schema FIELDS hash UNIQUE;
  `
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
    const result = await internalDb.query<SchemaRecord[]>(
      `SELECT hash, created_at FROM __schema ORDER BY created_at DESC LIMIT 1;`
    );

    // In surrealdb 1.x, query returns [result] where result is an array of rows
    if (result && result.length > 0 && Array.isArray(result[0]) && result[0].length > 0) {
      const firstRow = result[0][0] as SchemaRecord;
      return firstRow.hash === hash;
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
    await localDb.query(`REMOVE DATABASE ${database};`);
  } catch (error) {
    // Ignore error if database doesn't exist
  }
  await localDb.query(`DEFINE DATABASE ${database};`);
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
    await localDb.query(statement);
  }
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
  );
};

export async function runProvision(
  database: string,
  schemaSurql: string,
  databaseService: DatabaseService,
  logger: Logger,
  options: ProvisionOptions = {}
): Promise<void> {
  const { force = false } = options;

  logger.info("[Provisioning] Starting provision check...");

  const result = await databaseService.useInternal(async (db: Surreal) => {
    const schemaHash = await sha1(schemaSurql);
    const isUpToDate = await isSchemaUpToDate(db, schemaHash);
    let shouldMigrate = force || !isUpToDate;

    return { shouldMigrate, schemaHash, isUpToDate };
  });

  logger.debug(`[Provisioning] Schema hash: ${result.schemaHash}`);
  logger.debug(`[Provisioning] Schema up to date: ${result.isUpToDate}`);
  logger.debug(`[Provisioning] Should migrate: ${result.shouldMigrate}`);

  if (!result.shouldMigrate) {
    logger.info(
      "[Provisioning] Schema is up to date, skipping migration"
    );
    return;
  }

  logger.info("[Provisioning] Initializing internal database schema...");
  await databaseService.useInternal(async (db: Surreal) => {
    await initializeInternalDatabase(db);
  });

  logger.info("[Provisioning] Starting schema migration...");
  await databaseService.useLocal(async (db: Surreal) => {
    await dropMainDatabase(db, database);
    await provisionSchema(db, schemaSurql);
  });

  logger.debug("[Provisioning] Recording schema hash...");
  await databaseService.useInternal(async (db: Surreal) => {
    await recordSchemaHash(db, result.schemaHash);
  });

  logger.info(
    "[Provisioning] Database schema provisioned successfully"
  );
}
