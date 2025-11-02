import { Effect } from "effect";
import type { Surreal } from "surrealdb";

/**
 * Options for database provisioning
 */
export interface ProvisionOptions {
  /** Force re-provision even if schema already exists */
  force?: boolean;
  /** Custom schema content (optional) */
  customSchema?: string;
  /** Namespace for the database */
  namespace?: string;
  /** Main database name */
  database?: string;
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
export const sha1 = (str: string) =>
  Effect.tryPromise({
    try: async () => {
      const enc = new TextEncoder();
      const hash = await crypto.subtle.digest("SHA-1", enc.encode(str));
      return Array.from(new Uint8Array(hash))
        .map((v) => v.toString(16).padStart(2, "0"))
        .join("");
    },
    catch: (error) =>
      new Error(`Failed to compute SHA-1 hash: ${error}`),
  });

/**
 * Initializes the internal database with __schema table
 */
export const initializeInternalDatabase = (internalDb: Surreal) =>
  Effect.tryPromise({
    try: async () => {
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

/**
 * Checks if the current schema hash matches the stored hash
 */
export const isSchemaUpToDate = (internalDb: Surreal, hash: string) =>
  Effect.tryPromise({
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
    catch: (error) =>
      new Error(`Failed to check schema status: ${error}`),
  });

/**
 * Drops the main database and recreates it
 */
export const dropMainDatabase = (localDb: Surreal, database: string) =>
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
    catch: (error) =>
      new Error(`Failed to drop main database: ${error}`),
  });

/**
 * Provisions the schema by executing all SurrealQL statements
 */
export const provisionSchema = (localDb: Surreal, schemaContent: string) =>
  Effect.tryPromise({
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
    catch: (error) =>
      new Error(`Failed to provision schema: ${error}`),
  });

/**
 * Records the schema hash in the internal database
 */
export const recordSchemaHash = (internalDb: Surreal, hash: string) =>
  Effect.tryPromise({
    try: async () => {
      await internalDb.query(
        `UPSERT __schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
        { hash }
      );
      console.log("Schema hash recorded in internal database");
      return Effect.succeed(undefined);
    },
    catch: (error) =>
      new Error(`Failed to record schema hash: ${error}`),
  });

/**
 * Main provision function that orchestrates the provisioning process
 * This is the primary entry point for database schema provisioning
 */
export const provision = (
  context: ProvisionContext,
  options: ProvisionOptions = {}
) =>
  Effect.gen(function* (_) {
    const { internalDb, localDb, database, schema } = context;
    const { force = false, customSchema } = options;

    // Initialize internal database
    yield* _(initializeInternalDatabase(internalDb));

    // Use custom schema if provided
    const schemaContent = customSchema || schema;

    if (!schemaContent) {
      return yield* _(Effect.fail(new Error("No schema content available")));
    }

    // Compute schema hash
    const schemaHash = yield* _(sha1(schemaContent));

    // Check if migration is needed
    const isUpToDate = yield* _(isSchemaUpToDate(internalDb, schemaHash));
    const needsMigration = force || !isUpToDate;

    if (!needsMigration) {
      console.log("Schema is up to date, skipping provisioning...");
      return Effect.succeed(undefined);
    }

    if (!force) {
      console.log("Schema changed detected, migrating database...");
    } else {
      console.log("Force provisioning database...");
    }

    // Drop and recreate main database
    yield* _(dropMainDatabase(localDb, database));

    // Provision the new schema
    yield* _(provisionSchema(localDb, schemaContent));

    // Record the new schema hash
    yield* _(recordSchemaHash(internalDb, schemaHash));

    console.log("Database schema provisioned successfully");
    return Effect.succeed(undefined);
  }).pipe(
    Effect.catchAll((error) => {
      console.error("Failed to provision database schema:", error);
      return Effect.fail(error);
    })
  );

/**
 * Creates a provision context from configuration
 */
export const createProvisionContext = (
  internalDb: Surreal,
  localDb: Surreal,
  schema: string,
  namespace: string = "main",
  database: string = "main"
): ProvisionContext => ({
  internalDb,
  localDb,
  namespace,
  database,
  internalDatabase: `${database}__internal`,
  schema,
});
