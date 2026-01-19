import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal, Table } from 'surrealdb';

describe('SurrealDB Simple CRUD', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
    // Drop database clean before starting
    try {
      await db.query(`REMOVE DATABASE ${TEST_DB_CONFIG.database}`);
    } catch (e) {
      // Ignore if db doesn't exist
    }
    await db.query(`DEFINE DATABASE ${TEST_DB_CONFIG.database}`);
    await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
  });

  afterAll(async () => {
    await db.close();
  });

  test('should create and retrieve a record', async () => {
    // Removed invalid create call

    const created = await db.create(new Table('person')).content({
      title: 'Founder',
      name: {
        first: 'Tobie',
        last: 'Morgan Hitchcock',
      },
      marketing: true,
    });
    // expect(created[0]).toHaveProperty('id');
    // const id = created[0].id;
    // Actually, let's treat it as any to be safe and inspect
    const res = created as any;
    const id = Array.isArray(res) ? res[0].id : res.id;
    expect(id).toBeDefined();

    const selected = await db.select(id);
    const sel = selected as any;
    const item = Array.isArray(sel) ? sel[0] : sel;
    expect(item).toMatchObject({
      title: 'Founder',
      name: { first: 'Tobie' },
    });
  });

  test('should update a record', async () => {
    const created = await db.create(new Table('person')).content({
      name: 'Jane Doe',
    });
    const res = created as any;
    const person = Array.isArray(res) ? res[0] : res;

    // Use query for merge to be safe
    const updated = (await db
      .query(`UPDATE ${person.id} MERGE $data`, { data: { name: 'Jane Smith' } })
      .collect()) as any;

    const up = updated as any;
    const item =
      Array.isArray(up) && Array.isArray(up[0]) ? up[0][0] : Array.isArray(up) ? up[0] : up;
    expect(item.name).toBe('Jane Smith');
  });

  test('should delete a record', async () => {
    const createdDel = await db.create(new Table('person')).content({ name: 'To Be Deleted' });
    const res = createdDel as any;
    const person = Array.isArray(res) ? res[0] : res;

    await db.delete(person.id);

    // Verify deletion using a query to be unambiguous
    const checkQuery = await db.query('SELECT * FROM person WHERE id = $id', { id: person.id });
    const checkRes = checkQuery as any;

    // Result should be empty array or array with empty list
    const found =
      Array.isArray(checkRes) && checkRes[0] && checkRes[0].result
        ? checkRes[0].result
        : Array.isArray(checkRes)
          ? checkRes
          : [];

    expect(found.length).toBe(0);
  });
});
