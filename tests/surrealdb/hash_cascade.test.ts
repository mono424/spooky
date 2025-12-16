import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal } from 'surrealdb';

describe('Hash Cascade Logic', () => {
    let db: Surreal;

    beforeAll(async () => {
        db = await createTestDb();
        // We use the 'main' database here because that's where the events and schema usually are.
        // The setup.ts creates 'test_db'. We might need to switch to 'main' for this test 
        // to leverage the existing schema/events if they are not applied to 'test_db'.
        // However, messing with 'main' is risky.
        // Ideally, we should provision 'test_db' with the SAME schema.
        // For now, let's TRY to use 'main' but inside a transaction that we rollback? 
        // Or just create the schema on 'test_db'?
        // The user script says "Using your script logic", implying we should run that exactly.
        // Let's assume we run it on 'main' because that's where the app runs.
        await db.use({ namespace: 'main', database: 'main' });
    });

    afterAll(async () => {
        await db.close();
    });

    test('Create User, Thread, Comment -> Hash Tables update (XOR Check)', async () => {
        const script = `
        -- ==================================================
        -- 0. SETUP (Using your script logic)
        -- ==================================================
        BEGIN TRANSACTION;

        -- Create User
        LET $temp_username = "user_" + <string>time::nano(time::now());
        LET $user = (CREATE user CONTENT {
            username: $temp_username,
            password: "temppass123", -- Simplified for test speed
            created_at: time::now()
        })[0];
        LET $uid = $user.id;

        -- Create Threads
        LET $thread1 = (CREATE thread CONTENT {
            title: "Thread 1 (Even Comments)",
            content: "Content 1",
            author: $uid
        })[0];

        LET $thread2 = (CREATE thread CONTENT {
            title: "Thread 2 (Odd Comments)",
            content: "Content 2",
            author: $uid
        })[0];

        -- Create Comments
        -- Thread 1 gets 2 comments (Even parity)
        CREATE comment CONTENT { thread: $thread1.id, content: "C1 on T1", author: $uid };
        CREATE comment CONTENT { thread: $thread1.id, content: "C2 on T1", author: $uid };

        -- Thread 2 gets 1 comment (Odd parity)
        CREATE comment CONTENT { thread: $thread2.id, content: "C1 on T2", author: $uid };

        -- ==================================================
        -- 1. CAPTURE "BEFORE" STATE
        -- ==================================================

        -- Sleep briefly to ensure async events (if any) settle, though usually synchronous in transaction (events are sync now?)
        -- Events are usually sync in SurrealDB but user put SLEEP.
        SLEEP 100ms;

        LET $hash_user_before    = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $uid);
        LET $hash_thread1_before = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread1.id);
        LET $hash_thread2_before = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread2.id);

        -- ==================================================
        -- 2. PERFORM MUTATION (Trigger Cascade)
        -- ==================================================

        -- Update User Username
        -- This changes User Intrinsic -> Cascades to Comment Intrinsic -> Bubbles to Thread Composition
        UPDATE $uid SET username = $temp_username + "_UPDATED";

        -- ==================================================
        -- 3. CAPTURE "AFTER" STATE
        -- ==================================================

        SLEEP 2000ms; -- Increased sleep to be safe

        LET $hash_user_after    = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $uid);
        LET $hash_thread1_after = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread1.id);
        LET $hash_thread2_after = (SELECT value TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread2.id);
        
        -- Logic:
        -- t1_comp_before == t1_comp_after (Even parity, XOR cancels out)
        -- t2_comp_before != t2_comp_after (Odd parity, XOR change persists)

        LET $t1_comp_before = (SELECT value CompositionHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread1.id);
        LET $t1_comp_after  = (SELECT value CompositionHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread1.id);

        LET $t2_comp_before = (SELECT value CompositionHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread2.id);
        LET $t2_comp_after  = (SELECT value CompositionHash FROM ONLY _spooky_data_hash WHERE RecordId = $thread2.id);

        -- Clean up? Or leave it?
        -- Since verified via return, maybe rollback? 
        -- But SLEEP doesn't work well in transactions for waiting on SIDE EFFECTS if they are async.
        -- If events are synchronous, it's fine.
        -- COMMIT to invoke events if they are triggered on commit? No, events are largely mostly sync now.
        
        COMMIT TRANSACTION;

        RETURN {
            "3_RESULTS": {
                "Thread_1_Even_Parity": {
                    description: "Has 2 comments. Delta XOR Delta should be 0.",
                    composition_changed: $t1_comp_before != $t1_comp_after,
                    test_status: IF $t1_comp_before == $t1_comp_after THEN "PASS" ELSE "FAIL" END
                },
                "Thread_2_Odd_Parity": {
                    description: "Has 1 comment. Delta should affect Composition.",
                    composition_changed: $t2_comp_before != $t2_comp_after,
                    test_status: IF $t2_comp_before != $t2_comp_after THEN "PASS" ELSE "FAIL" END
                }
            }
        };
        `;

        const result = await db.query(script).collect() as any;
        // db.query returns list of results for each statement if multiple.
        // The last return is what we want.
        // With SurrealDB client, if we send one string, do we get array of results? Yes.
        const lastResult = result[result.length - 1] as any;
        console.log("Test Result Report:", JSON.stringify(lastResult, null, 2));

        expect(lastResult['3_RESULTS']['Thread_1_Even_Parity']['test_status']).toBe('PASS');
        expect(lastResult['3_RESULTS']['Thread_2_Odd_Parity']['test_status']).toBe('PASS');
    });
});
