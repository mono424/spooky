import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal, Table } from 'surrealdb';

describe('Intrinsic Hash Logic', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  test('Intrinsic Hash should update on content change', async () => {
    // 1. Create a user
    const tempUsername = 'intrinsic_test_' + Date.now();
    const created = await db.create(new Table('user')).content({
      username: tempUsername,
      password: 'password123',
      created_at: new Date(),
    });
    const res = created as any;
    const user = Array.isArray(res) ? res[0] : res;
    const userId = user.id;

    // Wait for hash change (polling)
    const waitForHashChange = async (
      initial: string,
      shouldChange: boolean = true,
      timeout = 5000
    ) => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        const q = (await db
          .query(
            `SELECT value intrinsicHash FROM ONLY _spooky_data_hash WHERE recordId = ${userId}`
          )
          .collect()) as any;
        const h = Array.isArray(q) ? q[0] : q;
        if (h) {
          if (shouldChange && h !== initial) return h;
          if (!shouldChange && h === initial) return h;
        }
        await new Promise((r) => setTimeout(r, 200));
      }
      const q = (await db
        .query(`SELECT value intrinsicHash FROM ONLY _spooky_data_hash WHERE recordId = ${userId}`)
        .collect()) as any;
      return Array.isArray(q) ? q[0] : q;
    };

    const initialHash = await waitForHashChange('', true); // wait for existence

    console.log('Initial Hash:', initialHash);
    expect(initialHash).toBeDefined();
    expect(initialHash).not.toBeNull();

    // 3. Update the user (change username)
    const updatedUsername = tempUsername + '_updated';
    await db.query(`UPDATE ${userId} SET username = '${updatedUsername}'`);

    // 4. Get new intrinsicHash
    const updatedHash = await waitForHashChange(initialHash, true);

    console.log('Updated Hash:', updatedHash);
    expect(updatedHash).toBeDefined();
    expect(updatedHash).not.toBe(initialHash);

    // 5. Revert the change
    await db.query(`UPDATE ${userId} SET username = '${tempUsername}'`);

    // 6. Get final intrinsicHash (Should match initial)
    const revertHash = await waitForHashChange(initialHash, false);

    console.log('Revert Hash:', revertHash);
    expect(revertHash).toBe(initialHash);
  }, 30000);
});
