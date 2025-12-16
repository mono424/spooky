import { Surreal } from 'surrealdb';

export const TEST_DB_CONFIG = {
  url: 'http://localhost:8000/rpc', // Using RPC for WebSocket, or http for HTTP
  namespace: 'test_ns',
  database: 'test_db',
  user: 'root',
  pass: 'root',
};

export async function createTestDb() {
  const db = new Surreal();
  await db.connect(TEST_DB_CONFIG.url);
  await db.signin({
    username: TEST_DB_CONFIG.user,
    password: TEST_DB_CONFIG.pass,
  });
  await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
  return db;
}

export async function clearTestDb(db: Surreal) {
  // Dangerous but needed for clean tests
  await db.query('REMOVE DATABASE test_db;');
  // Re-select?
  // await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
}
