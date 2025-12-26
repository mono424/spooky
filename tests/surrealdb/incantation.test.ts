import { createTestDb, clearTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal } from 'surrealdb';

describe('Spooky Incantations', () => {
    let db: Surreal;

    async function runQuery(query: string) {
        const results = await (db.query(query) as any).collect();
        // If results is an array of results (multi-statement), return them.
        // If it's a single result, wrap it? 
        // SurrealDB.js v1-v2 returns array of Result objects { result, status, time }.
        // If collecting, we might get [[res1], [res2]].
        return results;
    }

    beforeAll(async () => {
        db = await createTestDb();
    });

    afterAll(async () => {
        if (db) {
            await clearTestDb(db);
            await db.close();
        }
    });

    test('should track active threads via incantation', async () => {
        // 1. Prepare Query Plan for "Active Threads" using SQL
        // Matches threads where "id" starts with "thread:active"
        const sql = "SELECT * FROM thread WHERE id = 'thread:active*'";

        // 2. Register Incantation via Generated Function
        // Usage: fn::incantation::register({ id: '...', table: '...', query: 'SQL', ... })
        const registerQuery = `
            RETURN fn::incantation::register({
                id: "inc_active_threads",
                client_id: "client_1",
                items: [],
                paths: [],
                table: "thread",
                filter: {},
                query: "${sql}"
            });
        `;
        const regRes = await runQuery(registerQuery);
        // Expect a hash to be returned
        expect(regRes[0]).toBeDefined();
        console.log("Registered Incantation:", regRes[0]);

        // DEBUG: Check what was actually saved
        const checkQuery = `
            LET $inc = (SELECT * FROM _spooky_incantation WHERE Id = 'inc_active_threads')[0];
            LET $state = fn::dbsp::get_state();
            RETURN { incantation: $inc, state: $state };
        `;
        const checkRes = await runQuery(checkQuery);
        console.log("Post-registration check:", JSON.stringify(checkRes[checkRes.length - 1], null, 2));

        // 3. Create Threads (Triggers _spooky_thread_mutation -> dbsp::ingest -> Update Incantation)
        
        // A. Create Active Thread (Should match)
        const createRes = await runQuery(`
            CREATE thread:active_new CONTENT {
                active: true,
                title: 'SurrealDB 2.0',
                content: 'Some content',
                author: user:1
            }
        `);

        // DEBUG: Check state after CREATE
        const afterCreateQuery = `
            LET $inc = (SELECT * FROM _spooky_incantation WHERE Id = 'inc_active_threads')[0];
            LET $state = fn::dbsp::get_state();
            RETURN { incantation: $inc, state: $state };
        `;
        const afterCreateRes = await runQuery(afterCreateQuery);
        console.log("After CREATE check:", JSON.stringify(afterCreateRes[afterCreateRes.length - 1], null, 2));

        // B. Create Inactive Thread (Should NOT match)
        await runQuery(`
            CREATE thread:draft_new CONTENT {
                active: false,
                title: 'Draft Post',
                content: 'Draft content',
                author: user:1
            }
        `);

        // 4. Verify Incantation State Loop
        // The event should have updated _spooky_incantation table
        // We look for the record with Id "view_active_threads" (ID from Plan) OR "inc_active_threads"?
        // Wait. `fn::dbsp::register_query` uses `$after.Id` from `_spooky_incantation`.
        // `fn::incantation::register` sets `_spooky_incantation.Id` to "inc_active_threads".
        // So the Plan ID should match that!
        // But in step 1, I named the plan "view_active_threads".
        // `fn::dbsp::register_query` (in Lib.rs) takes `id` and `plan_json`.
        // It injects `id` into the plan. So the plan's inner ID is overwritten or ignored?
        // Checking Reg Query in Lib.rs:
        // `parsed.id = id;` -> YES. It overwrites.
        // So the WASM module will key the view by "inc_active_threads".
        
        // Check local state in DB
        const incQuery = `SELECT * FROM _spooky_incantation WHERE Id = 'inc_active_threads'`;
        const incRes = await runQuery(incQuery);
        const incRecord = incRes[0][0];

        expect(incRecord).toBeDefined();
        console.log("Incantation Record:", JSON.stringify(incRecord, null, 2));

        // Verify Hash and Tree are present
        expect(incRecord.Hash).toBeDefined();
        expect(incRecord.Tree).toBeDefined();
        expect(incRecord.Tree.ids).toBeDefined();
        
        // Should contain only thread:active_new
        // Note: Tree.ids is just an array of IDs.
        // We expect IDs to be FULL Record Strings "thread:active_new" (because we used table matches)
        // Or "active_new"? 
        // Based on logic "table:id", it should be "thread:active_new".
        expect(incRecord.Tree.ids).toContain("thread:active_new");
        expect(incRecord.Tree.ids).not.toContain("thread:draft_new");

        // 5. Update: Delete the active thread
        await runQuery(`DELETE thread:active_new`);
        
        const incRes2 = await runQuery(incQuery);
        const incRecord2 = incRes2[0][0];
        console.log("Incantation Record After Delete:", JSON.stringify(incRecord2, null, 2));

        // DEBUG: Check State After Delete to see zset
        const stateRes2 = await runQuery('SELECT * FROM _spooky_module_state:singleton');
        const state2 = stateRes2[stateRes2.length-1];
        if (state2) {
             console.log("State2 Object:", JSON.stringify(state2, null, 2));
             if (state2.State && state2.State.db && state2.State.db.tables && state2.State.db.tables.thread) {
                 console.log("State After Delete Check (ZSet):", JSON.stringify(state2.State.db.tables.thread.zset, null, 2));
             }
        }

        // Should be empty
        // Note: Tree.ids might be missing or empty array depending on impl
        if (incRecord2.Tree.ids) {
            expect(incRecord2.Tree.ids.length).toBe(0);
        } else {
            // Implicitly empty
             expect(true).toBe(true);
        }
        
        // Verify Hash changed
        expect(incRecord2.Hash).not.toBe(incRecord.Hash);
    });

    test('should update incantation hash deterministically (Reversion Test)', async () => {
        // 1. Register a simple view
        const sql = "SELECT * FROM det_item WHERE id = 'det:item*'";
        
        const registerQuery = `
            RETURN fn::incantation::register({
                id: "inc_det",
                client_id: "client_1",
                items: [],
                paths: [],
                table: "det_item",
                filter: {},
                query: "${sql}"
            });
        `;
        const regRes = await runQuery(registerQuery);
        expect(regRes[0]).toBeDefined();

        // Helper to get current Hash
        const getHash = async () => {
            const r = await runQuery(`SELECT Hash FROM ONLY _spooky_incantation WHERE Id = 'inc_det'`);
            return r[0] ? r[0].Hash : null;
        };

        // H1: Initial Hash (Empty)
        const H1 = await getHash();
        expect(H1).toBeDefined();

        // 2. Ingest Item A -> H2
        await runQuery(`CREATE det:item_a CONTENT { val: 1 }`);
        const H2 = await getHash();
        
        expect(H2).not.toBe(H1);
        console.log("H1 (Empty):", H1);
        console.log("H2 (Item A):", H2);

        // 3. Revert: Delete Item A -> Should return to H1
        await runQuery(`DELETE det:item_a`);
        const H3 = await getHash();
        console.log("H3 (Reverted):", H3);

        expect(H3).toBe(H1);

        // 4. Determinism: CREATE Item A again (Same content) -> H4
        // Note: ID must be same. Content same.
        await runQuery(`CREATE det:item_a CONTENT { val: 1 }`);
        const H4 = await getHash();
        console.log("H4 (Re-Item A):", H4);
        
        expect(H4).toBe(H2);
        
        // 5. Different Item B -> H5
        await runQuery(`CREATE det:item_b CONTENT { val: 2 }`);
        // Note: det:item_b matches prefix det:item*.
        const H5 = await getHash();
        expect(H5).not.toBe(H2);
        
        // 6. Delete B -> Back to H4 (== H2)
        await runQuery(`DELETE det:item_b`);
        const H6 = await getHash();
        expect(H6).toBe(H4);
    });
});
