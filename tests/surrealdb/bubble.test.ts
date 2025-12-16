import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal, Table } from 'surrealdb';

describe('Bubble Hash Logic', () => {
    let db: Surreal;

    beforeAll(async () => {
        db = await createTestDb();
        // db is already configured for test_ns/test_db by setup.ts
        // await db.use({ namespace: 'main', database: 'main' }); // Use the db configured in setup
    });

    afterAll(async () => {
        await db.close();
    });

    test('Hash should bubble up from Child to Parent', async () => {
        // 0. Create a User (Author)
        const user = await db.create(new Table('user')).content({
            username: "bubble_user_" + Date.now(),
            password: "password",
            created_at: new Date()
        });
        const userRes = user as any;
        const userRec = Array.isArray(userRes) ? userRes[0] : userRes;
        const userId = userRec.id;

        // 1. Create a Thread (Parent)
        const threadIdVal = "bubble_thread_" + Date.now();
        const thread = await db.create(new Table('thread')).content({
            title: "Parent Thread " + threadIdVal,
            content: "Thread Content",
            author: userId, 
            created_at: new Date()
        });
        const threadRes = thread as any;
        const threadRec = Array.isArray(threadRes) ? threadRes[0] : threadRes;
        const threadId = threadRec.id;

        // Wait for initial hash calc
        await new Promise(r => setTimeout(r, 2000));
        
        // Get initial Thread Hash (Composition)
        // Adjust field name if necessary based on schema. Assuming 'CompositionHash' or 'TotalHash'.
        // From hash_cascade.test.ts logic, we checked TotalHash. Let's check CompositionHash if available, or TotalHash.
        // The previous test looked at TotalHash.
        const getThreadHash = async () => {
             const res = await db.query(`SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = ${threadId}`).collect() as any;
             return Array.isArray(res) ? res[0] : res;
        };

        const initialThreadHash = await getThreadHash();
        console.log("Initial Thread Hash:", initialThreadHash);
        expect(initialThreadHash).toBeDefined();

        // 2. Create a Comment (Child) linked to Thread
        const commentIdVal = "bubble_comment_" + Date.now();
        const comment = await db.create(new Table('comment')).content({
            content: "Child Content 1",
            thread: threadId,
            author: userId,
            created_at: new Date()
        });
        const commentRes = comment as any;
        const commentRec = Array.isArray(commentRes) ? commentRes[0] : commentRes;
        const commentId = commentRec.id;

        // Wait for bubble up
        await new Promise(r => setTimeout(r, 3000));

        // 3. Verify Thread Hash Changed
        const threadHashAfterChild = await getThreadHash();
        console.log("Thread Hash After Child:", threadHashAfterChild);
        
        // Creating a child SHOULD update the parent's hash if the schema is set up for it (e.g. via relations or graph traversal in hash calc).
        // If the hash is purely content-based intrinsic, it won't change. 
        // But "Bubble" implies aggregation.
        expect(threadHashAfterChild).not.toBe(initialThreadHash);

        // 4. Update Child
        await db.query(`UPDATE ${commentId} SET content = 'Child Content Updated'`);
        
        // Wait for bubble up
        await new Promise(r => setTimeout(r, 3000));

        // 5. Verify Thread Hash Changed Again
        const threadHashAfterUpdate = await getThreadHash();
        console.log("Thread Hash After Update:", threadHashAfterUpdate);
        expect(threadHashAfterUpdate).not.toBe(threadHashAfterChild);

        // 6. Delete Child
        await db.delete(commentId);
         
        // Wait for bubble up
        await new Promise(r => setTimeout(r, 3000));

        // 7. Verify Thread Hash Reverted (or changed back to something similar to initial)
        // It might not be EXACTLY initial if initial included "0 comments" vs "had comments then deleted".
        // But usually it should match if it's a sum/xor of children.
        const threadHashAfterDelete = await getThreadHash();
        console.log("Thread Hash After Delete:", threadHashAfterDelete);
        expect(threadHashAfterDelete).toBe(initialThreadHash);
    }, 45000); // Long timeout for sleeps
});
