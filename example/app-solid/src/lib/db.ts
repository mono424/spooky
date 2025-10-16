import { SyncedDb, type SyncedDbConfig } from "db-solid";

// Database configuration
const dbConfig: SyncedDbConfig = {
  localDbName: "thread-app-local",
  storageStrategy: "indexeddb",
  namespace: "main",
  database: "thread_app",
  // Uncomment and configure these for remote sync
  // remoteUrl: "http://localhost:8000",
  // token: "your-auth-token-here"
};

// Create and export the database instance
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

// Export database instance for use in components
export { db };
