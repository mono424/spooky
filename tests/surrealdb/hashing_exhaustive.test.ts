
import { createTestDb } from './setup';
import { Surreal, Table } from 'surrealdb';

let db: Surreal;

describe('Exhaustive Hashing Tests', () => {
    beforeAll(async () => {
        db = await createTestDb();
    });

    afterAll(async () => {
        await db.close();
    });

    const getHash = async (recordId: string) => {
        const res = await db.query(`SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = ${recordId}`).collect() as any;
        return Array.isArray(res) ? res[0] : res;
    };

    const waitForHashChange = async (recordId: string, initialTotalHash: string, timeout = 5000) => {
        const start = Date.now();
        while (Date.now() - start < timeout) {
            const current = await getHash(recordId);
            if (current && current.TotalHash !== initialTotalHash) return current;
            await new Promise(r => setTimeout(r, 100));
        }
        return await getHash(recordId);
    };

    test('Scenario: Thread Lifecycle with Multiple Comments', async () => {
        // 0. User Creation (for valid author)
        const user = await db.create(new Table('user')).content({
            username: "exhaustive_user_" + Date.now(),
            password: "password",
            created_at: new Date()
        });
        const userId = (user as any)[0].id;

        // 1. Thread Creation
        const thread = await db.create(new Table('thread')).content({
            title: "Exhaustive Thread",
            content: "Root Content",
            author: userId
        });
        const threadId = (thread as any)[0].id;
        
        const hash1 = await getHash(threadId);
        expect(hash1.IntrinsicHash).toBeDefined();
        
        expect(hash1.CompositionHash).toBeDefined();
        // 1. Dependencies (Comments) - initially empty/undefined (not checked yet)
        
        // 2. References (Author) - MUST be present in CompositionHash now
        expect(hash1.CompositionHash).toHaveProperty('author');
        expect(typeof hash1.CompositionHash.author).toBe('string');
        // It should NOT be empty hash if the user exists
        expect(hash1.CompositionHash.author.length).toBeGreaterThan(0);

        // Composition should have _xor
        expect(hash1.CompositionHash._xor).toBeDefined(); 
        
        // 3. Intrinsic Fields - MUST NOT be in CompositionHash
        expect(hash1.CompositionHash).not.toHaveProperty('title');
        
        // 2. Child Creation (Comment A)
        const commentA = await db.create(new Table('comment')).content({
            content: "Comment A",
            thread: threadId,
            author: userId
        });
        const commentAId = (commentA as any)[0].id;

        const hash2 = await waitForHashChange(threadId, hash1.TotalHash);
        expect(hash2.TotalHash).not.toBe(hash1.TotalHash);
        expect(hash2.CompositionHash).toHaveProperty('comment');
        expect(hash2.CompositionHash).not.toHaveProperty('content'); // No intrinsic fields
        
        // 3. Multiple Children (Comment B)
        const commentB = await db.create(new Table('comment')).content({
            content: "Comment B",
            thread: threadId,
            author: userId
        });
        const commentBId = (commentB as any)[0].id;

        const hash3 = await waitForHashChange(threadId, hash2.TotalHash);
        expect(hash3.TotalHash).not.toBe(hash2.TotalHash);
        
        // 4. Child Update (Comment A)
        await db.query(`UPDATE ${commentAId} SET content = "Comment A Modified"`);
        
        const hash4 = await waitForHashChange(threadId, hash3.TotalHash);
        expect(hash4.TotalHash).not.toBe(hash3.TotalHash);
        
        // 5. Intrinsic Change (Thread Title) - Should NOT affect CompositionHash
        // Capture Composition State
        const compositionBefore = JSON.stringify(hash4.CompositionHash);
        
        await db.query(`UPDATE ${threadId} SET title = "Exhaustive Thread Renamed"`);
        
        const hash5 = await waitForHashChange(threadId, hash4.TotalHash);
        expect(hash5.TotalHash).not.toBe(hash4.TotalHash);
        expect(hash5.IntrinsicHash).not.toBe(hash4.IntrinsicHash);
        
        // CRITICAL: CompositionHash must remain EXACTLY the same
        expect(JSON.stringify(hash5.CompositionHash)).toBe(compositionBefore);
        
        // 6. Child Deletion (Comment B)
        await db.query(`DELETE ${commentBId}`);
        
        const hash6 = await waitForHashChange(threadId, hash5.TotalHash);
        expect(hash6.TotalHash).not.toBe(hash5.TotalHash);
        
        // 7. Revert State Check (Optional/Approximate)
        // If we revert Comment A to original, and delete Comment B... 
        // We can't easily revert to *exact* hash1 because Intrinsic changed.
        
    }, 60000);
});
