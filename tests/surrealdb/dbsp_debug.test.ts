import { Surreal } from 'surrealdb';
import { createTestDb, clearTestDb } from './setup';

describe('DBSP Debug', () => {
  let db: Surreal;

  async function runQuery(query: string) {
    return await (db.query(query) as any).collect();
  }

  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    if (db) await clearTestDb(db);
    if (db) await db.close();
  });

  test('DEBUG: Probe register_view', async () => {
    try {
      // mod::dbsp::register_view(id: String, plan: Value)
      console.log('DEBUG: Calling register_view...');
      const q = `RETURN mod::dbsp::register_view("debug_id", "debug_plan")`;
      const res = await runQuery(q);
      console.log('DEBUG: Register success:', JSON.stringify(res));
    } catch (e: any) {
      console.log('DEBUG: Register failed: ' + e.message);
      throw e;
    }
  });
});
