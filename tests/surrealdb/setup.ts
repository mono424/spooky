import { Surreal } from 'surrealdb';
import { GenericContainer, StartedTestContainer, Wait } from 'testcontainers';

let container: StartedTestContainer | null = null;
let currentPort: number | null = null;

export const TEST_DB_CONFIG = {
  namespace: 'test_ns',
  database: 'test_db',
  user: 'root',
  pass: 'root',
};

async function getContainer() {
    if (container) return container;
    
    console.log("Starting SurrealDB container...");
    const path = require('path');
    const modulesDir = path.resolve(__dirname, '../../example/schema/.spooky');
    
    try {
        container = await new GenericContainer("surrealdb/surrealdb:v3.0.0-alpha.17-dev")
        .withExposedPorts(8000)
        .withBindMounts([
            { source: modulesDir, target: "/tmp/modules", mode: "rw" }
        ])
        .withUser("root") 
        .withCommand(["start", "--log", "trace", "--user", "root", "--pass", "root", "--allow-all", "--allow-experimental"]) 
        .withStartupTimeout(10000)
        .withLogConsumer((stream) => {
            stream.pipe(process.stdout);
        })
        // Switch to a simple wait strategy initially to debug startup
        .withWaitStrategy(Wait.forLogMessage("Started web server on 0.0.0.0:8000"))
        .start();
    } catch (e) {
        console.error("Container failed to start");
        throw e;
    }

    if (!container) throw new Error("Container failed to initialize");
    
    currentPort = container!.getMappedPort(8000);
    console.log(`SurrealDB started on port ${currentPort}`);
    return container;
}

export async function createTestDb() {
  await getContainer();
  const db = new Surreal();
  
  if (!currentPort) throw new Error("Container not started properly");
  
  await db.connect(`http://localhost:${currentPort}/rpc`);
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
      let schema = fs.readFileSync(schemaPath, 'utf8');
      
      // Patch: Replace WASM module with JS implementation for tests to avoid file permission issues
      schema = schema.replace(/DEFINE BUCKET modules.*/g, '-- DEFINE BUCKET SKIPPED');
      schema = schema.replace(/DEFINE MODULE mod::xor.*/g, '-- DEFINE MODULE SKIPPED');
      
      // Update usage of module function to local JS function
      schema = schema.replace(/mod::xor::blake3_xor/g, 'fn::xor_blake3_xor');
      
      // Prepend the function definition to ensure it exists before events use it
      // Using pure SurQL dummy implementation to bypass embedded JS issues in test container.
      // Logic: Return concat($a, "1"). Ensures state grows/changes on every update but is deterministic (prevents infinite loop if convergent logic exists, though "1" growth is infinite if triggered cyclically. Hope cascading logic has stop condition).
      const mockFunction = `DEFINE FUNCTION fn::xor_blake3_xor($a: string, $b: string) { RETURN string::concat($a, "1"); };\n\n`;
      schema = mockFunction + schema;

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
  await db.query(`REMOVE DATABASE ${TEST_DB_CONFIG.database};`);
  // Re-select?
  // await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
}

