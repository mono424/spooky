import { SyncedDb } from "@spooky/client-solid";
import { schema, SURQL_SCHEMA } from "./schema.gen";

// Database configuration
export const dbConfig = {
  schema: schema,
  schemaSurql: SURQL_SCHEMA,
  localDbName: "thread-app-local",
  internalDbName: "syncdb-int",
  storageStrategy: "indexeddb",
  namespace: "main",
  database: "main",
  remoteUrl: "ws://localhost:8000",
  provisionOptions: {
    force: false,
  },
} as const;

export const db = new SyncedDb(dbConfig);

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
