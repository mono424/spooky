import { createTestDb } from './setup';
import { Surreal, Table } from 'surrealdb';

let db: Surreal;

describe('Schema Defaults', () => {
  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  test('Should match DEFAULT {} for Where field in _spooky_query_lookup', async () => {
    // Create a record without 'Where' field
    const created = await db.create(new Table('_spooky_query_lookup')).content({
      IncantationId: 'test_incantation',
      Table: 'thread',
      // Where: omitted
      SortFields: [],
      SortDirections: [],
    });

    const id = (created as any)[0].id;

    // Fetch it back
    const result = await db.select(id);
    const record = result as any;

    console.log('Created Record:', record);

    // Verify 'Where' is {} (empty object) and NOT undefined or null
    expect(record.Where).toBeDefined();
    expect(record.Where).toEqual({});
  });
});
