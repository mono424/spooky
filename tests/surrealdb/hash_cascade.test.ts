
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
        const commentId = (comment as any)[0].id;
        console.log("Comment Created:", commentId);

        // 4. Capture Initial Hash
        const getHash = async (recordId: string) => {
             const res = await db.query(`SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = ${recordId}`).collect() as any;
             return Array.isArray(res) ? res[0] : res;
        };
        const initialHash = await getHash(threadId);
        // const initialUserHash = await getHash(userId);

        console.log("Initial Thread Hash:", initialHash.TotalHash);
        // console.log("Initial User Hash:", initialUserHash?.TotalHash);
        
        expect(initialHash.TotalHash).toBeDefined();

        // 4. Update Comment -> Triggers Cascade (Bubble)
        // Note: Updating User (Reference) does not change Thread Hash unless schema embeds User. 
        // Updating Comment (Child) creates a delta that propagates to Thread (Parent).
        try {
            console.log("Updating Comment...");
            // Use query to control update precisely
            const result = await db.query(`UPDATE ${commentId} SET content = "updated_comment_${Date.now()}"`).collect();
            console.log("Comment Update Result:", JSON.stringify(result));
        } catch (e: any) {
            console.error("Comment Update Failed:", e);
            throw e;
        }

        // Wait for cascade
        const waitForHashChange = async (initial: string, timeout = 5000) => {
            const start = Date.now();
            while (Date.now() - start < timeout) {
                const current = await getHash(threadId);
                if (current && current.TotalHash !== initial) return current;
                await new Promise(r => setTimeout(r, 200));
            }
            return await getHash(threadId);
        }

        const finalHash = await waitForHashChange(initialHash.TotalHash);
        
        console.log("Final Thread Hash:", finalHash.TotalHash);

        // Assert record exists and has hashes
        expect(finalHash.IntrinsicHash).toBeDefined();
        expect(finalHash.CompositionHash).toBeDefined();
        expect(finalHash.TotalHash).toBeDefined();

        // CRITICAL ASSERTION: TotalHash MUST change
        expect(finalHash.TotalHash).not.toBe(initialHash.TotalHash);
    }, 60000);
});
