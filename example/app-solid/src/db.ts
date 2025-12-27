import { SyncedDb } from '@spooky/client-solid';
import { schema, SURQL_SCHEMA } from './schema.gen';

// Database configuration
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
let isInitialized = false;

export async function initDatabase(): Promise<void> {
  if (isInitialized) return;

  try {
    console.log('Initializing database...');
    await db.init();
    isInitialized = true;
    console.log('Database initialized successfully');
  } catch (error) {
    console.error('Failed to initialize database:', error);
    throw error;
  }
}

// Database instance is already exported above
