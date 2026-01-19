import { createTestDb } from './setup';
import { SURQL_SCHEMA } from '../.spooky/client_schema';
import { Surreal, Table } from 'surrealdb';

describe('Client Spooky Events', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
    // Use a separate namespace/database for client testing to avoid conflicts
    try {
      await db.query('REMOVE DATABASE test_client');
    } catch (e) {}
    await db.query('DEFINE DATABASE test_client');
    await db.use({ namespace: 'test_client', database: 'test_client' });

    // Load the client schema (which includes the client-specific events)
    // Load the client schema
    try {
      await db.query(SURQL_SCHEMA);
    } catch (e: any) {
      console.error('Schema load failed:', e);
      if (e.message && e.message.includes('Parse error')) {
        console.error('Parse error possibly at:', e.message);
      }
      // Try to run a simple query to see if DB is alive
      try {
        await db.query('INFO FOR DB;');
        console.log('DB is alive.');
      } catch (e2) {
        console.error('DB check failed:', e2);
      }
      throw e;
    }
  });

  afterAll(async () => {
    // Optional cleanup
  });

  test('CREATE sets IsDirty=true and calculates IntrinsicHash', async () => {
    const userRes = (await db.create(new Table('user')).content({
      username: 'client_user_' + Date.now(),
    })) as any;

    const user = Array.isArray(userRes) ? userRes[0] : userRes;

    expect(user).toBeDefined();
    expect(user.id).toBeDefined();

    // Verify _spooky_data_hash
    const result: any = await (
      db.query('SELECT * FROM _spooky_data_hash WHERE RecordId = $id', { id: user.id }) as any
    ).collect();
    // Assuming result structure matches bubble.test.ts experience or standard v2
    // If result is [ActionResult], then result[0].result is records.
    // However, if we blindly check [0], let's try to be robust.
    // If result is simple array of records (because helper handles it?), let's log if needed.
    // But for now, let's assume it returns standard raw response array.

    // Inspect result[0]
    const queryResult = result[0];
    const hashRecord =
      queryResult && queryResult.result && Array.isArray(queryResult.result)
        ? queryResult.result[0]
        : Array.isArray(queryResult)
          ? queryResult[0]
          : queryResult;

    expect(hashRecord).toBeDefined();
    expect(hashRecord.IntrinsicHash).toBeDefined();
    expect(typeof hashRecord.IntrinsicHash).toBe('string');
    expect(hashRecord.IntrinsicHash.length).toBeGreaterThan(0);

    // Check flags
    expect(hashRecord.IsDirty).toBe(true);
    expect(hashRecord.PendingDelete).toBe(false);

    // Check Composition/Total are default/empty for client
    // NONE in SurQL maps to undefined in JS
    expect(hashRecord.TotalHash).toBeFalsy();
  });

  test('UPDATE sets IsDirty=true and updates IntrinsicHash', async () => {
    const userRes = (await db.create(new Table('user')).content({
      username: 'client_user_update_' + Date.now(),
    })) as any;
    const user = Array.isArray(userRes) ? userRes[0] : userRes;

    // Get initial hash
    const res1: any = await (
      db.query('SELECT * FROM _spooky_data_hash WHERE RecordId = $id', { id: user.id }) as any
    ).collect();
    const qr1 = res1[0];
    const hash1 =
      qr1 && qr1.result && Array.isArray(qr1.result)
        ? qr1.result[0]
        : Array.isArray(qr1)
          ? qr1[0]
          : qr1;

    // Reset dirty flag
    await db.query('UPDATE _spooky_data_hash SET IsDirty = false WHERE RecordId = $id', {
      id: user.id,
    });

    // Perform Update: Change username to trigger hash change
    await db.query('UPDATE $id MERGE $content', {
      id: user.id,
      content: { username: user.username + '_updated' },
    });

    // Verify hash changed and IsDirty is true
    const res2: any = await (
      db.query('SELECT * FROM _spooky_data_hash WHERE RecordId = $id', { id: user.id }) as any
    ).collect();
    const qr2 = res2[0];
    const hash2 =
      qr2 && qr2.result && Array.isArray(qr2.result)
        ? qr2.result[0]
        : Array.isArray(qr2)
          ? qr2[0]
          : qr2;

    expect(hash2.IntrinsicHash).not.toEqual(hash1.IntrinsicHash);
    expect(hash2.IsDirty).toBe(true);
  });

  test('DELETE sets PendingDelete=true', async () => {
    const userRes = (await db.create(new Table('user')).content({
      username: 'client_user_delete_' + Date.now(),
    })) as any;
    const user = Array.isArray(userRes) ? userRes[0] : userRes;

    // Verify created
    const res1: any = await (
      db.query('SELECT * FROM _spooky_data_hash WHERE RecordId = $id', { id: user.id }) as any
    ).collect();
    expect(res1[0]).toBeDefined();

    // Perform Delete
    await db.delete(user.id);

    // Verify _spooky_data_hash STILL EXISTS and has PendingDelete=true
    const res2: any = await (
      db.query('SELECT * FROM _spooky_data_hash WHERE RecordId = $id', { id: user.id }) as any
    ).collect();

    const qr = res2[0];
    const hashRecord =
      qr && qr.result && Array.isArray(qr.result) ? qr.result[0] : Array.isArray(qr) ? qr[0] : qr;

    expect(hashRecord).toBeDefined(); // Should not be deleted
    expect(hashRecord.PendingDelete).toBe(true);
  });
});
