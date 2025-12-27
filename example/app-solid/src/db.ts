import { SyncedDb } from '@spooky/client-solid';
import { schema, SURQL_SCHEMA } from './schema.gen';

// Database configuration
export const dbConfig = {
  schema: schema,
  schemaSurql: SURQL_SCHEMA,
  database: {
    namespace: 'main',
    database: 'main',
    endpoint: 'ws://localhost:8000/rpc',
    // auth: { ... } // If needed later
  },
} as const;

export const db = new SyncedDb(dbConfig);

// Initialize the database
let initializationPromise: Promise<void> | null = null;

export function initDatabase(): Promise<void> {
  if (initializationPromise) return initializationPromise;

  initializationPromise = (async () => {
    try {
      console.log('Initializing database...');
      await db.init();
      console.log('Database initialized successfully');
    } catch (error) {
      console.error('Failed to initialize database:', error);
      initializationPromise = null; // Allow retrying if it failed
      throw error;
    }
  })();

  return initializationPromise;
}

// Database instance is already exported above
