import { SyncedDb, type SyncedDbConfig } from "db-solid";
import { type Schema, SURQL_SCHEMA } from "./schema.gen";
export type { Schema };

// Database configuration
export const dbConfig: SyncedDbConfig<Schema> = {
  schema: SURQL_SCHEMA,
  localDbName: "thread-app-local",
  internalDbName: "syncdb-int",
  storageStrategy: "indexeddb",
  namespace: "main",
  database: "thread_app",
  remoteUrl: "http://localhost:8000",
  tables: ["user", "thread", "comment"],
};

// Create and export the database instance with proper schema types
export const db = new SyncedDb<Schema>(dbConfig);

// Initialize the database
let isInitialized = false;

export async function initDatabase(): Promise<void> {
  if (isInitialized) return;

  try {
    console.log("Initializing database...");
    await db.init();
    isInitialized = true;
    console.log("Database initialized successfully");
  } catch (error) {
    console.error("Failed to initialize database:", error);
    throw error;
  }
}

// Database instance is already exported above
