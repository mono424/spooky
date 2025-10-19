import type { Surreal } from "surrealdb";

async function sha1(str: string) {
  const enc = new TextEncoder();
  const hash = await crypto.subtle.digest("SHA-1", enc.encode(str));
  return Array.from(new Uint8Array(hash))
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("");
}

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

export class SchemaProvisioner {
  private internalDb: Surreal;
  private localDb: Surreal;
  private schema: string;
  private namespace: string;
  private database: string;
  private internalDatabase: string;
  constructor(
    internalDb: Surreal,
    localDb: Surreal,
    namespace: string = "main",
    database: string = "main",
    schema: string
  ) {
    this.internalDb = internalDb;
    this.localDb = localDb;
    this.namespace = namespace;
    this.database = database;
    this.internalDatabase = `${database}__internal`;
    this.schema = schema;
  }

  /**
   * Provisions the database with the schema from schema.surql
   * Automatically migrates by dropping and recreating if schema changes
   */
  async provision(options: ProvisionOptions = {}): Promise<void> {
    try {
      await this.initializeInternalDatabase();

      const { force = false, customSchema } = options;

      // Use custom schema if provided
      const schemaContent = customSchema || this.schema;

      if (!schemaContent) {
        throw new Error("No schema content available");
      }

      const schemaHash = await sha1(schemaContent);

      // Check current schema in internal database
      const needsMigration =
        force || !(await this.isSchemaUpToDate(schemaHash));

      if (!needsMigration) {
        console.log("Schema is up to date, skipping provisioning...");
        return;
      }

      if (!force) {
        console.log("Schema changed detected, migrating database...");
      } else {
        console.log("Force provisioning database...");
      }

      // Switch to main database and drop everything
      await this.dropMainDatabase();

      // Provision the new schema
      await this.provisionSchema(schemaContent);

      // Record the new schema hash in internal database
      await this.recordSchemaHash(schemaHash);

      console.log("Database schema provisioned successfully");
    } catch (error) {
      console.error("Failed to provision database schema:", error);
      throw error;
    }
  }

  /**
   * Checks if the current schema matches the stored hash
   */
  private async isSchemaUpToDate(hash: string): Promise<boolean> {
    try {
      const [result] = await this.internalDb
        .query(
          `SELECT hash, created_at FROM __schema ORDER BY created_at DESC LIMIT 1;`
        )
        .collect<[{ hash: string; created_at: string }[]]>();
      // Result is an array of query results
      if (result.length > 0) {
        return result[0].hash === hash;
      }
      return false;
    } catch (error) {
      console.error("Error checking schema up to date:", error);
      console.log("Internal database not initialized yet");
      return false;
    }
  }

  /**
   * Drops all tables and definitions in the main database
   */
  private async dropMainDatabase(): Promise<void> {
    try {
      console.log("Dropping main database...");
      await this.localDb.query(`REMOVE DATABASE ${this.database};`);
    } catch (error) {}
    try {
      await this.localDb.query(`DEFINE DATABASE ${this.database};`);
      console.log("Main database dropped successfully");
    } catch (error) {
      console.error("Error creating main database:", error);
      throw error;
    }
  }

  /**
   * Provisions the schema by executing all statements
   */
  private async provisionSchema(schemaContent: string): Promise<void> {
    try {
      console.log("Provisioning new schema...");

      // Split into statements and execute them individually
      const statements = schemaContent
        .split(";")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);

      for (const statement of statements) {
        try {
          await this.localDb.query(statement);
          console.info(`Executed statement:\n${statement}`);
        } catch (err: any) {
          console.error(`Error executing statement: ${statement}`);
          throw err;
        }
      }

      console.log("Schema provisioned successfully");
    } catch (error) {
      console.error("Error provisioning schema:", error);
      throw error;
    }
  }

  private async initializeInternalDatabase(): Promise<void> {
    await this.internalDb
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
  }

  /**
   * Records the schema hash in the internal database
   */
  private async recordSchemaHash(hash: string): Promise<void> {
    try {
      // Insert new schema record
      await this.internalDb.query(
        `CREATE __schema SET hash = $hash, created_at = time::now();`,
        { hash }
      );

      console.log("Schema hash recorded in internal database");
    } catch (error) {
      console.error("Error recording schema hash:", error);
      throw error;
    }
  }

  /**
   * Gets the current schema content
   */
  getSchema(): string {
    return this.schema;
  }

  /**
   * Updates the schema content
   */
  setSchema(schema: string): void {
    this.schema = schema;
  }
}
