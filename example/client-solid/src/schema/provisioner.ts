import type { Surreal } from "surrealdb";
import schemaSurql from "database/schema.surql?raw";

export interface ProvisionOptions {
  /** Force re-provision even if schema already exists */
  force?: boolean;
  /** Custom schema content (optional) */
  customSchema?: string;
}

export class SchemaProvisioner {
  private db: Surreal;
  private schema: string;

  constructor(db: Surreal, customSchema?: string) {
    this.db = db;
    this.schema = customSchema || schemaSurql;
  }

  /**
   * Provisions the database with the schema from schema.surql
   */
  async provision(options: ProvisionOptions = {}): Promise<void> {
    try {
      const { force = false, customSchema } = options;

      // Use custom schema if provided
      const schemaContent = customSchema || this.schema;

      if (!schemaContent) {
        throw new Error("No schema content available");
      }

      // Check if schema is already provisioned (unless force is true)
      if (!force) {
        const isProvisioned = await this.isSchemaProvisioned();
        if (isProvisioned) {
          console.log("Schema already provisioned, skipping...");
          return;
        }
      }

      console.log("Provisioning database schema...");

      // Execute the schema
      await this.db.query(schemaContent);

      console.log("Database schema provisioned successfully");
    } catch (error) {
      console.error("Failed to provision database schema:", error);
      throw error;
    }
  }

  /**
   * Checks if the schema is already provisioned
   */
  private async isSchemaProvisioned(): Promise<boolean> {
    try {
      // Check if key tables exist
      const result = await this.db.query(`
        SELECT COUNT() FROM INFORMATION_SCHEMA.tables 
        WHERE name IN ['user', 'message', 'friend_request', 'thread', 'comment']
      `);

      const count = result[0]?.result?.[0] || 0;
      return count >= 5;
    } catch (error) {
      // If we can't check, assume not provisioned
      return false;
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
