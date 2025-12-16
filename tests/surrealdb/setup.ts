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
  
  // Clean start for every test execution
  // Must select namespace first to perform operations
  await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
  
  // Then we can manage databases. Wait, REMOVE DATABASE requires being in a namespace?
  // Yes.
  try {
      await db.query(`REMOVE DATABASE ${TEST_DB_CONFIG.database};`);
  } catch (e) { /* ignore if not exists */ }
  
  await db.query(`DEFINE DATABASE ${TEST_DB_CONFIG.database};`);
  await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
  
  // Load generated schema
  const fs = require('fs');
  const path = require('path');
  const schemaPath = path.resolve(__dirname, '../../tests/schema.gen.surql');
  if (fs.existsSync(schemaPath)) {
      console.log("Loading schema from:", schemaPath);
      const schema = fs.readFileSync(schemaPath, 'utf8');
      if (schema.includes("THROW")) console.log("Schema contains THROW");
      else console.log("Schema does NOT contain THROW");
      const q = await db.query(schema) as any;
      const results = (q && typeof q.collect === 'function') ? await q.collect() : q;
      // Check for errors in results
      if (Array.isArray(results)) {
        for (const res of results) {
            if (res.status === 'ERR') {
                console.error("Schema Load Error:", res);
                throw new Error("Schema Load Failed: " + JSON.stringify(res));
            }
        }
      } else {
        console.warn("Schema query returned non-array:", results);
      }
  } else {
      console.warn("Schema file not found at " + schemaPath + ". Tests might fail.");
  }
  
  return db;
}

export async function clearTestDb(db: Surreal) {
  // Dangerous but needed for clean tests
  await db.query('REMOVE DATABASE test_db;');
  // Re-select?
  // await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
}
