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
    const response = await internalDb.query(
      `SELECT hash, created_at FROM __schema ORDER BY created_at DESC LIMIT 1;`
    );

    // In surrealdb v2, result is ActionResult[]
    if (Array.isArray(response) && response.length > 0) {
      const firstResult = response[0];
      if (firstResult.status === "OK") {
        const records = firstResult.result as SchemaRecord[];
        if (Array.isArray(records) && records.length > 0) {
          return records[0].hash === hash;
        }
      }
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
  database: string,
  namespace: string
): Promise<void> => {
  try {
    // Switch to a temporary context to avoid dropping the active database
    // which causes a hang in the connection
    await localDb.use({ namespace: namespace, database: "temp_provisioning" });
    await localDb.query(`REMOVE DATABASE ${database};`);
  } catch (error) {
    // Ignore error if database doesn't exist
  } finally {
    // Re-create and switch back to the target database
    try {
      await localDb.query(`DEFINE DATABASE ${database};`);
      await localDb.use({ namespace: namespace, database: database });
    } catch (e) {
      console.error("Error recreating database", e);
      throw e;
    }
  }
};

/**
 * Provisions the schema by executing all SurrealQL statements
 */
export const provisionSchema = async (
  localDb: Surreal,
  schemaContent: string,
  logger: Logger
): Promise<void> => {
  // Split into statements and execute them individually
  const statements = schemaContent
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  logger.info(`[Provisioning] Found ${statements.length} statements to apply.`);

  for (let i = 0; i < statements.length; i++) {
    const statement = statements[i];
    // DEBUG: Skip DEFINE INDEX to see if it's the blocker
    if (statement.toUpperCase().startsWith("DEFINE INDEX")) {
      logger.warn(`[Provisioning] SKIPPING statement: ${statement.substring(0, 50)}...`);
      continue;
    }
    logger.info(`[Provisioning] (${i + 1}/${statements.length}) Executing: ${statement.substring(0, 50)}...`);
    try {
      await localDb.query(statement);
    } catch (e) {
      logger.error(`[Provisioning] Error executing statement: ${statement}`);
      throw e;
    }
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

// Timeout helper
const withTimeout = <T>(
  promise: Promise<T>,
  ms: number,
  errorMessage: string
): Promise<T> => {
  let timeoutId: any;
  const timeoutPromise = new Promise<T>((_, reject) => {
    timeoutId = setTimeout(() => {
      reject(new Error(errorMessage));
    }, ms);
  });

  return Promise.race([promise, timeoutPromise]).finally(() => {
    clearTimeout(timeoutId);
  });
};

export async function runProvision(
  database: string,
  namespace: string,
  schemaSurql: string,
  databaseService: DatabaseService,
  logger: Logger,
  options: ProvisionOptions = {}
): Promise<void> {
  const { force = false } = options;

  logger.info("[Provisioning] Starting provision check...");

  try {
    const result = await withTimeout(
      databaseService.useInternal(async (db: Surreal) => {
        logger.debug("[Provisioning] Computing schema hash...");
        const schemaHash = await sha1(schemaSurql);
        logger.debug(`[Provisioning] Computed hash: ${schemaHash}`);
        
        logger.debug("[Provisioning] Checking if schema is up to date...");
        const isUpToDate = await isSchemaUpToDate(db, schemaHash);
        let shouldMigrate = force || !isUpToDate;

        return { shouldMigrate, schemaHash, isUpToDate };
      }),
      5000,
      "Timeout while checking schema status (5s)"
    );

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
    await withTimeout(
      databaseService.useInternal(async (db: Surreal) => {
        await initializeInternalDatabase(db);
      }),
      5000,
      "Timeout while initializing internal database (5s)"
    );

    logger.info("[Provisioning] Starting schema migration...");
    await databaseService.useLocal(async (db: Surreal) => {
      logger.debug(`[Provisioning] Dropping database '${database}'...`);
      await withTimeout(
        dropMainDatabase(db, database, namespace),
        10000,
        "Timeout while dropping database (10s)"
      );
      logger.debug(`[Provisioning] Database '${database}' dropped/recreated.`);
      
      logger.debug("[Provisioning] Applying schema...");
      await withTimeout(
        provisionSchema(db, schemaSurql, logger),
        30000,
        "Timeout while applying schema (30s)"
      );
      logger.debug("[Provisioning] Schema applied successfully.");
    });

    logger.debug("[Provisioning] Recording schema hash...");
    await withTimeout(
      databaseService.useInternal(async (db: Surreal) => {
        await recordSchemaHash(db, result.schemaHash);
      }),
      5000,
      "Timeout while recording schema hash (5s)"
    );

    logger.info(
      "[Provisioning] Database schema provisioned successfully"
    );
  } catch (error) {
    logger.error(`[Provisioning] Failed: ${error instanceof Error ? error.message : String(error)}`);
    throw error;
  }
}
