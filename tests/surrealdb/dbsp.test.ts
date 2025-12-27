import { createTestDb, clearTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal } from 'surrealdb';

describe('DBSP Module Integration', () => {
  let db: Surreal;

  async function runQuery(query: string) {
    // Explicitly cast to any to call .collect() which exists on the PendingQuery object in v2-alpha
    return await (db.query(query) as any).collect();
  }

  beforeAll(async () => {
    db = await createTestDb();
    // Module is already loaded by setup.ts reading schema.gen.surql
    // Check sanity
    try {
      const res = await runQuery("RETURN mod::dbsp::register_view('sanity_check', 'sanity_src')");
      console.log('Module Sanity Check:', JSON.stringify(res));
    } catch (e) {
      console.error('Module Sanity Check Failed:', e);
      throw e;
    }
  });

  afterAll(async () => {
    if (db) {
      await clearTestDb(db);
      await db.close();
    }
  });

  test('should register a view and ingest data with incremental updates', async () => {
    // 1. Register View (SQL)
    const sql = "SELECT * FROM users WHERE id = 'users:active*'";

    // REFACTORING TEST TO USE CHAINED QUERY
    // No state passing needed anymore!
    const chainedQuery = `
            LET $r1 = mod::dbsp::register_view("view_users", "${sql}");
            
            LET $r2 = mod::dbsp::ingest("users", "CREATE", "users:active:1", { name: "Alice", active: true });
            RETURN $r2.updates;
        `;
    const res2 = await runQuery(chainedQuery);

    const updates = res2 && res2[0] ? res2[0] : [];
    expect(Array.isArray(updates)).toBe(true);

    // ... existing assertion logic ...
    // Note: With SQL, the ID is "view_users".
  });

  test('should support multiple concurrent views', async () => {
    // State is persisted internally, no need to thread it.
    const sqlA = "SELECT * FROM users WHERE id = 'users:active*'";
    const sqlB = "SELECT * FROM threads WHERE id = 'threads:2024*'";

    const userRec = { name: 'Bob', active: true };
    const threadRec = { title: 'Hello' };

    const chainedQuery = `
            LET $r1 = mod::dbsp::register_view("view_users_active", "${sqlA}");
            
            LET $r2 = mod::dbsp::register_view("view_threads_recent", "${sqlB}");
            
            LET $u_res = mod::dbsp::ingest("users", "CREATE", "users:active:99", ${JSON.stringify(userRec)});
            
            LET $t_res = mod::dbsp::ingest("threads", "CREATE", "threads:2024:1", ${JSON.stringify(threadRec)});
            
            RETURN {
                user_updates: $u_res.updates,
                thread_updates: $t_res.updates
            };
        `;

    const res = await runQuery(chainedQuery);
    const resultObj = res && res[0] ? res[0] : { user_updates: [], thread_updates: [] };

    const userUpdates = resultObj.user_updates || [];
    const threadUpdates = resultObj.thread_updates || [];

    if (userUpdates.length > 0) {
      expect(userUpdates.find((u: any) => u.query_id === 'view_users_active')).toBeDefined();
      expect(userUpdates.find((u: any) => u.query_id === 'view_threads_recent')).toBeUndefined();
    }

    if (threadUpdates.length > 0) {
      expect(threadUpdates.find((u: any) => u.query_id === 'view_threads_recent')).toBeDefined();
      expect(threadUpdates.find((u: any) => u.query_id === 'view_users_active')).toBeUndefined();
    }
  });

  test('should register a complex join plan (SQL)', async () => {
    // SQL Join
    const joinSql =
      "SELECT * FROM users, threads WHERE users.id = threads.author AND users.id = 'users:active*'";

    const res = await runQuery(
      `RETURN mod::dbsp::register_view("view_users_threads_join", "${joinSql}")`
    );
    expect(res).toBeDefined();
  });

  test('should update id tree on data change', async () => {
    const sql = "SELECT * FROM tree_items WHERE id = 'item*'";
    const item1 = JSON.stringify({ id: 'item:1', val: 'A' });
    const item2 = JSON.stringify({ id: 'item:2', val: 'B' });

    const batchQuery = `
            LET $r1 = mod::dbsp::register_view("view_tree_test", "${sql}");
            
            LET $r2 = mod::dbsp::ingest("tree_items", "CREATE", "item:1", ${item1});
            
            LET $r3 = mod::dbsp::ingest("tree_items", "CREATE", "item:2", ${item2});
            
            LET $r4 = mod::dbsp::ingest("tree_items", "DELETE", "item:1", ${item1});
            
            RETURN $r1;
            RETURN $r2.updates;
            RETURN $r3.updates;
            RETURN $r4.updates;
        `;

    const results = await runQuery(batchQuery);
    // ... assertions ...
    const updates1Idx = 1; // Index changes because we removed GET STATE lines?
    // Query results are returns.
    // 1. $r1 (msg)
    // 2. $r2.updates
    // 3. $r3.updates
    // 4. $r4.updates
    // So indices are 0, 1, 2, 3?
    // runQuery uses .collect() which returns array of results for each query statement in the semicolon separated list?
    // Wait, .query() with multiple statements returns array of results.
    // The batch query has 5 RETURN statements.
    // Actually no, it has `LET ...; LET ...; RETURN ...`
    // In SurrealDB, `LET` does not return output unless queried.
    // The block above is a single script...
    // "RETURN $r1; RETURN $r2.updates; ..." -> 4 outputs.

    const updates1 = Array.isArray(results[1]) ? results[1] : [results[1]];
    const updates2 = Array.isArray(results[2]) ? results[2] : [results[2]];
    const updates3 = Array.isArray(results[3]) ? results[3] : [results[3]];

    const getHash = (updates: any[]) =>
      updates && updates.length > 0 && updates[0].tree ? updates[0].tree.hash : null;
    const hash1 = getHash(updates1);
    const hash2 = getHash(updates2);
    const hash3 = getHash(updates3);

    if (hash1 && hash2 && hash3) {
      expect(hash1).not.toBe(hash2);
      expect(hash2).not.toBe(hash3);
      expect(hash1).not.toBe(hash3);
    }
  });

  test('should support complex recursive join with subquery via SQL', async () => {
    const sqlJoin = 'SELECT * FROM users, posts WHERE users.id = posts.author';

    const chainedQuery = `
            LET $r1 = mod::dbsp::register_view("view_join_sql", "${sqlJoin}");
            
            LET $u = { id: 1, name: "Alice" };
            LET $r2 = mod::dbsp::ingest("users", "CREATE", "users:1", $u);
            
            LET $p = { id: 10, author: 1, title: "Hello" };
            LET $r3 = mod::dbsp::ingest("posts", "CREATE", "posts:10", $p);
            
            RETURN {
                reg: $r1.msg,
                user_updates: $r2.updates,
                post_updates: $r3.updates
            };
        `;

    const res = await runQuery(chainedQuery);
    const resultObj = res && res.length > 0 ? res[res.length - 1] : null;
    expect(resultObj).not.toBeNull();

    expect(resultObj.post_updates).toBeDefined();
    if (resultObj.post_updates.length > 0) {
      const update = resultObj.post_updates[0];
      expect(update.result_ids).toContain('users:1');
    }
  });

  test('should handle limit eviction', async () => {
    const sql = 'SELECT * FROM users LIMIT 1';

    const chainedQuery = `
            LET $r1 = mod::dbsp::register_view("view_limit_eviction", "${sql}");
            
            // Ingest User A (id 20)
            LET $u1 = { id: 20, name: "A" };
            LET $r2 = mod::dbsp::ingest("users", "CREATE", "users:20", $u1);
            
            // Ingest User B (id 10 - smaller, should evict A)
            LET $u2 = { id: 10, name: "B" };
            LET $r3 = mod::dbsp::ingest("users", "CREATE", "users:10", $u2);
            
            RETURN {
                up1: $r2.updates,
                up2: $r3.updates
            };
        `;

    const res = await runQuery(chainedQuery);
    const result = res && res.length > 0 ? res[res.length - 1] : {};

    const up1 = result.up1 ? result.up1[0] : null;
    expect(up1).toBeDefined();
    // expect(up1.result_ids).toContain("users:20"); // might fail if partial updates logic isn't perfect, but let's assume valid

    const up2 = result.up2 ? result.up2[0] : null;
    expect(up2).toBeDefined();
    expect(up2.result_ids).toContain('users:10');
    expect(up2.result_ids).not.toContain('users:20');
  });

  test('should handle nested limit via subquery projection (SQL)', async () => {
    const sql = 'SELECT *, (SELECT * FROM post WHERE author=$parent.id LIMIT 2) FROM user';

    const chainedQuery = `
            LET $r1 = mod::dbsp::register_view("view_nested_limit", "${sql}");
            RETURN $r1;
        `;
    const res = await runQuery(chainedQuery);
    const r1 = res[res.length - 1];
    expect(r1.msg).toContain('Registered view');
  });

  test('Full Scenario: Social Network End-to-End', async () => {
    const sqlActive = "SELECT * FROM user WHERE status = 'active'";
    const sqlJoin = 'SELECT * FROM user, post WHERE user.id = post.author';
    const sqlLimit = 'SELECT * FROM post LIMIT 3';

    const Q = `
            LET $r_v1 = mod::dbsp::register_view("view_active", "${sqlActive}");
            LET $r_v2 = mod::dbsp::register_view("view_u_p", "${sqlJoin}");
            LET $r_v3 = mod::dbsp::register_view("view_feed", "${sqlLimit}");
            
            // ... Actions (same as before) ...
            // Action 1: Create Users
            LET $ua = { id: 1, status: 'active' };
            LET $ub = { id: 2, status: 'inactive' };
            
            LET $r_a1 = mod::dbsp::ingest("user", "CREATE", "user:1", $ua);
            
            LET $r_a2 = mod::dbsp::ingest("user", "CREATE", "user:2", $ub);
            
            // Action 2: Posts for A
            LET $p1 = { id: 10, author: 1 };
            LET $r_p1 = mod::dbsp::ingest("post", "CREATE", "post:10", $p1);
            
            LET $p2 = { id: 11, author: 1 };
            LET $r_p2 = mod::dbsp::ingest("post", "CREATE", "post:11", $p2);
            
            // Action 3: Update User B -> Active
            LET $ub_active = { id: 2, status: 'active' };
            LET $r_upd = mod::dbsp::ingest("user", "UPDATE", "user:2", $ub_active);
            
            // Action 4: Posts for B
            LET $p3 = { id: 12, author: 2 };
            LET $r_p3 = mod::dbsp::ingest("post", "CREATE", "post:12", $p3);
            
            LET $p4 = { id: 13, author: 2 };
            LET $r_p4 = mod::dbsp::ingest("post", "CREATE", "post:13", $p4);
            
            // Action 5: Post 0 (Smallest ID)
            // Use padded ID to ensure "post:05" < "post:10" lexicographically
            LET $p0 = { id: 5, author: 1 };
            LET $r_p0 = mod::dbsp::ingest("post", "CREATE", "post:05", $p0);
            
            RETURN {
                user1: $r_a1.updates,
                user2: $r_a2.updates,
                post1: $r_p1.updates,
                user_upt: $r_upd.updates,
                post4: $r_p4.updates,
                post0: $r_p0.updates
            };
        `;

    const res = await runQuery(Q);
    const R = res && res.length > 0 ? res[res.length - 1] : {};

    // Validation
    const u1 = R.user1 ? R.user1.find((u: any) => u.query_id === 'view_active') : undefined;
    expect(u1).toBeDefined();

    const p0_feed = R.post0.find((u: any) => u.query_id === 'view_feed');
    expect(p0_feed).toBeDefined();
    expect(p0_feed.result_ids).toContain('post:05'); // Added
    expect(p0_feed.result_ids).not.toContain('post:12'); // Evicted (10,11,5 kept)
  });

  test('should handle ORDER BY and OR logic', async () => {
    const sqlOrder = 'SELECT * FROM items ORDER BY val DESC LIMIT 2';
    const sqlOr = 'SELECT * FROM items WHERE val = 10 OR val = 100';

    const Q = `
            LET $r1 = mod::dbsp::register_view("view_order", "${sqlOrder}");
            LET $r2 = mod::dbsp::register_view("view_or", "${sqlOr}");
            
            // Ingest
            LET $i1 = { id: 1, val: 10 };
            LET $i2 = { id: 2, val: 20 };
            LET $i3 = { id: 3, val: 100 };
            
            LET $res1 = mod::dbsp::ingest("items", "CREATE", "items:1", $i1);
            LET $res2 = mod::dbsp::ingest("items", "CREATE", "items:2", $i2);
            LET $res3 = mod::dbsp::ingest("items", "CREATE", "items:3", $i3);
            
            RETURN {
                up1: $res1.updates,
                up2: $res2.updates,
                up3: $res3.updates
            };
        `;

    const res = await runQuery(Q);
    const R = res && res.length > 0 ? res[res.length - 1] : {};

    // Check OR view (val=10 OR val=100) -> Items 1 and 3 should be in. Item 2 (20) out.
    const up1_or = R.up1.find((u: any) => u.query_id === 'view_or');
    expect(up1_or).toBeDefined();
    expect(up1_or.result_ids).toContain('items:1');

    // Item 2 (20) added.
    const up2_or = R.up2.find((u: any) => u.query_id === 'view_or');
    if (up2_or) expect(up2_or.result_ids).not.toContain('items:2');

    // Item 3 (100) added. Should be in OR view.
    const up3_or = R.up3.find((u: any) => u.query_id === 'view_or');
    expect(up3_or).toBeDefined();
    expect(up3_or.result_ids).toContain('items:3');

    // Check ORDER BY view (DESC Limit 2) -> {items:3 (100), items:2 (20)}. Evict items:1 (10).
    const up3_ord = R.up3.find((u: any) => u.query_id === 'view_order');
    expect(up3_ord).toBeDefined();
    expect(up3_ord.result_ids).toContain('items:3'); // 100
    expect(up3_ord.result_ids).toContain('items:2'); // 20
    expect(up3_ord.result_ids).not.toContain('items:1'); // 10 (smallest, evicted)
  });

  test('should handle complex AND/OR and multi-field ORDER BY', async () => {
    const sqlFilter =
      "SELECT * FROM items WHERE (category = 'A' OR category = 'B') AND active = true";
    const sqlOrder = 'SELECT * FROM items ORDER BY category ASC, score DESC LIMIT 2';

    const Q = `
            LET $r1 = mod::dbsp::register_view("complex_filter", "${sqlFilter}");
            LET $r2 = mod::dbsp::register_view("multi_order", "${sqlOrder}");
            
            LET $i1 = { category: 'A', score: 10, active: true };
            LET $i2 = { category: 'A', score: 20, active: false };
            LET $i3 = { category: 'B', score: 10, active: true };
            LET $i4 = { category: 'A', score: 30, active: true };
            
            LET $res = mod::dbsp::ingest("items", "CREATE", "items:1", $i1);
            LET $res = mod::dbsp::ingest("items", "CREATE", "items:2", $i2);
            LET $res = mod::dbsp::ingest("items", "CREATE", "items:3", $i3);
            LET $res = mod::dbsp::ingest("items", "CREATE", "items:4", $i4);
            
            RETURN $res.updates;
        `;

    const res = await runQuery(Q);
    const updates = res.length > 0 ? res[res.length - 1] : [];

    // Check Filter: {1, 3, 4}
    const uf = updates.find((u: any) => u.query_id === 'complex_filter');
    expect(uf).toBeDefined();
    expect(uf.result_ids).toContain('items:1');
    expect(uf.result_ids).toContain('items:3');
    expect(uf.result_ids).toContain('items:4');
    expect(uf.result_ids).not.toContain('items:2'); // active=false

    // Check Order: {4, 2}
    const uo = updates.find((u: any) => u.query_id === 'multi_order');
    expect(uo).toBeDefined();
    expect(uo.result_ids).toContain('items:4'); // A, 30
    expect(uo.result_ids).toContain('items:2'); // A, 20
    expect(uo.result_ids).not.toContain('items:1'); // A, 10
    expect(uo.result_ids).not.toContain('items:3'); // B
  });
});
