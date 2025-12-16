
import { createTestDb } from './setup';
import { Surreal, Table } from 'surrealdb';

let db: Surreal;

describe('Hash Cascade Logic', () => {
    beforeAll(async () => {
        db = await createTestDb();
        // db.use is handled in setup for test_db
    });

    afterAll(async () => {
        await db.close();
    });

    test('Should execute cascade update step-by-step', async () => {
        // 1. Create User
        const user = await db.create(new Table('user')).content({
            username: "cascade_user_" + Date.now(),
            password: "password",
            created_at: new Date()
        });
        const userId = (user as any)[0].id;
        console.log("User Created:", userId);

        // 2. Create Thread
        const thread = await db.create(new Table('thread')).content({
            title: "Cascade Thread",
            content: "Content",
            author: userId
        });
        const threadId = (thread as any)[0].id;
        console.log("Thread Created:", threadId);

        // 3. Create Comment
        const comment = await db.create(new Table('comment')).content({
            content: "Cascade Comment",
            thread: threadId,
            author: userId
        });
        console.log("Comment Created");

        // 4. Update User -> Triggers Cascade
        try {
            console.log("Updating User...");
            // Use query to control update precisely
            const result = await db.query(`UPDATE ${userId} SET username = "updated_user_${Date.now()}"`).collect();
            console.log("User Update Result:", JSON.stringify(result));
        } catch (e: any) {
            console.error("User Update Failed:", e);
            throw e;
        }

        // 5. Check Hashes (if update succeeded)
        const hash = await db.query(`SELECT * FROM _spooky_data_hash WHERE RecordId = ${threadId}`).collect() as any[];
        console.log("Thread Hash Check:", JSON.stringify(hash, null, 2));

        // Assert record exists and has hashes
        expect(hash[0].length).toBeGreaterThan(0);
        const record = hash[0][0];
        expect(record.IntrinsicHash).toBeDefined();
        expect(record.CompositionHash).toBeDefined();
        expect(record.TotalHash).toBeDefined();
    }, 60000);
});
