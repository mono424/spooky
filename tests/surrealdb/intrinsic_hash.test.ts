import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal, Table } from 'surrealdb';

describe('Intrinsic Hash Logic', () => {
    let db: Surreal;

    beforeAll(async () => {
        db = await createTestDb();
        // Use 'main' to leverage existing events/schema
        await db.use({ namespace: 'main', database: 'main' });
    });

    afterAll(async () => {
        await db.close();
    });

    test('Intrinsic Hash should update on content change', async () => {
        // 1. Create a user
        const tempUsername = "intrinsic_test_" + Date.now();
        const created = await db.create(new Table('user')).content({
            username: tempUsername,
            password: "password123",
            created_at: new Date()
        });
        const res = created as any;
        const user = Array.isArray(res) ? res[0] : res;
        const userId = user.id;

        // Wait for potential async event processing
        await new Promise(r => setTimeout(r, 2000));

        // 2. Get initial IntrinsicHash
        const initialHashQuery = await db.query(`SELECT value IntrinsicHash FROM ONLY _spooky_data_hash WHERE RecordId = ${userId}`).collect() as any;
        const initialHash = Array.isArray(initialHashQuery) ? initialHashQuery[0] : initialHashQuery;
        
        console.log("Initial Hash:", initialHash);
        expect(initialHash).toBeDefined();
        expect(initialHash).not.toBeNull();


        // 3. Update the user (change username)
        const updatedUsername = tempUsername + "_updated";
        await db.query(`UPDATE ${userId} SET username = '${updatedUsername}'`);

        // Wait for processing
        await new Promise(r => setTimeout(r, 2000));

        // 4. Get new IntrinsicHash
        const updatedHashQuery = await db.query(`SELECT value IntrinsicHash FROM ONLY _spooky_data_hash WHERE RecordId = ${userId}`).collect() as any;
        const updatedHash = Array.isArray(updatedHashQuery) ? updatedHashQuery[0] : updatedHashQuery;

        console.log("Updated Hash:", updatedHash);
        expect(updatedHash).toBeDefined();
        expect(updatedHash).not.toBe(initialHash);

        // 5. Revert the change
        await db.query(`UPDATE ${userId} SET username = '${tempUsername}'`);

        // Wait for processing
        await new Promise(r => setTimeout(r, 2000));

        // 6. Get final IntrinsicHash (Should match initial)
        const revertHashQuery = await db.query(`SELECT value IntrinsicHash FROM ONLY _spooky_data_hash WHERE RecordId = ${userId}`).collect() as any;
        const revertHash = Array.isArray(revertHashQuery) ? revertHashQuery[0] : revertHashQuery;

        console.log("Revert Hash:", revertHash);
        // Note: created_at didn't change, so restoring username should restore the hash IF ONLY username/password/created_at are hashed.
        // If 'updated_at' is implicitly tracked and hashed, this might fail. 
        // Assuming deterministic hashing of content.
        expect(revertHash).toBe(initialHash);
    }, 30000);
});
