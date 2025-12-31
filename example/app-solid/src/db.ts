import { SyncedDb, SyncedDbConfig } from '@spooky/client-solid';
import { schema, SURQL_SCHEMA } from './schema.gen';

// Database configuration
export const dbConfig = {
  logLevel: 'trace',
  schema: schema,
  schemaSurql: SURQL_SCHEMA,
  database: {
    namespace: 'main',
    database: 'main',
    endpoint: 'ws://localhost:8666/rpc',
    // auth: { ... } // If needed later
  },
} satisfies SyncedDbConfig<typeof schema>;

export const db = new SyncedDb(dbConfig);

// Initialize the database
let initializationPromise: Promise<void> | null = null;

export function initDatabase(): Promise<void> {
  if (initializationPromise) return initializationPromise;

  initializationPromise = (async () => {
    try {
      await db.init();
    } catch (error) {
      initializationPromise = null; // Allow retrying if it failed
      throw error;
    }
  })();

  return initializationPromise;
}

// Database instance is already exported above
