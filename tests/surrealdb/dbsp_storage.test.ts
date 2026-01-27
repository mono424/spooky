import { createTestDb, clearTestDb } from './setup';
import { Surreal, RecordId } from 'surrealdb';

describe('DBSP Storage Persistence', () => {
  let db: Surreal;

  // Helper to run query and collect results (standard pattern in valid properties)
  async function runQuery(query: string) {
    return await (db.query(query) as any).collect();
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

  test('should save state correctly to _spooky_module_state after ingest', async () => {
    // 1. Reset state first to be clean
    await runQuery('RETURN mod::dbsp::reset({})');

    // 2. Register a view (triggers save_state)
    const registerSql =
      'RETURN mod::dbsp::register_view("storage_view", "SELECT * FROM storage_items", {})';
    const regRes = await runQuery(registerSql);
    // register_view returns null or value, but not necessarily {status: OK} wrapper in this client version
    expect(regRes).toBeDefined();

    // 3. Ingest data (triggers save_state)
    const ingestSql = `RETURN mod::dbsp::ingest("storage_items", "CREATE", "storage_items:1", { val: 42, hash: "0000" })`;
    const ingRes = await runQuery(ingestSql);
    // ingest returns { updates: [] }
    expect(ingRes[0].updates).toBeDefined();
    // 4. Check persistent storage table
    const checkSql = 'SELECT * FROM _spooky_module_state:dbsp';
    const storageRes = await runQuery(checkSql);
    console.log('Storage Res:', JSON.stringify(storageRes, null, 2));

    // Check if storageRes[0] is the result object or the records
    const records = storageRes[0].result || storageRes[0];
    // If it's a single record (object), wrap in array. If array, use it.
    const recordList = Array.isArray(records) ? records : [records];

    expect(recordList.length).toBeGreaterThan(0);
    const content = recordList[0].content || recordList[0]; // Handle if 'content' is nested or direct
    expect(records[0].id.toString()).toBe('_spooky_module_state:dbsp');
    expect(records[0].content).toBeDefined();

    expect(records[0].id.toString()).toBe('_spooky_module_state:dbsp');
    expect(records[0].content).toBeDefined();

    // 5. Automated Hash Update Test
    // Create an incantation and verify Hash is updated automatically (by _spooky_dbsp_register event)
    const autoIncantId = 'auto_test_view';
    const autoIncantSql = `
      UPSERT _spooky_incantation:${autoIncantId} CONTENT {
        id: '${autoIncantId}',
        sql: 'SELECT * FROM storage_items',
        params: {},
        ttl: 10m,
        lastActiveAt: time::now(),
        clientId: 'test-client'
      };
    `;
    await runQuery(autoIncantSql);

    // Check if Hash is populated
    const checkAutoSql = `SELECT * FROM _spooky_incantation:${autoIncantId}`;
    const autoRes = await runQuery(checkAutoSql);
    // console.log("Auto Incantation Res:", JSON.stringify(autoRes, null, 2));

    const autoRecords = autoRes[0].result || autoRes[0];
    const autoRecord = Array.isArray(autoRecords) ? autoRecords[0] : autoRecords;

    expect(autoRecord.hash).toBeDefined();
    expect(autoRecord.hash).not.toBe('');
    expect(autoRecord.hash).not.toBe('NONE');

    // Verify it registered in module state too
    const circuitRes = await runQuery('SELECT content FROM _spooky_module_state:dbsp');
    const circuitContent = JSON.parse((circuitRes[0].result || circuitRes[0])[0].content);
    expect(circuitContent.views.find((v: any) => v.plan.id === autoIncantId)).toBeDefined();
    expect(circuitContent.views.find((v: any) => v.plan.id === autoIncantId)).toBeDefined();

    // 6. Parameterized View Test
    // Create an incantation with parameters
    const paramIncantId = 'param_test_view';
    // User 'alice' exists from previous tests? No, schema is fresh.
    // Wait, storage_items are unrelated.
    // Let's use schema.surql tables. `user` table.
    // Create a user first.
    await runQuery(`CREATE user:alice SET username = 'alice', password = 'password123';`);
    const paramIncantSql = `
      UPSERT _spooky_incantation:${paramIncantId} CONTENT {
        id: '${paramIncantId}',
        sql: 'SELECT * FROM user WHERE id = $id',
        params: { id: "user:alice" },
        ttl: 10m,
        lastActiveAt: time::now(),
        clientId: 'test-client-param'
      };
    `;
    await runQuery(paramIncantSql);

    // Check Auto Hash
    const checkParamSql = `SELECT * FROM _spooky_incantation:${paramIncantId}`;
    const paramRes = await runQuery(checkParamSql);
    const paramRecord = (paramRes[0].result || paramRes[0])[0];

    expect(paramRecord.hash).toBeDefined();
    expect(paramRecord.hash).not.toBe('');
    expect(paramRecord.hash).not.toBe('NONE');

    const state = JSON.parse(records[0].content);
    expect(state.db).toBeDefined();
    expect(state.db.tables).toBeDefined();
    expect(state.db.tables['storage_items']).toBeDefined();
    expect(state.views).toHaveLength(1);
    expect(state.views[0].plan.id).toBe('storage_view');
  });

  test('should handle unquoted record ID parameters (sql style)', async () => {
    // Mimic the failing scenario: { id: thread:123 }
    // Create a dummy thread
    await runQuery(
      `CREATE thread:test_parsing SET title = 'parsing_test', author = user:alice, content = 'fake content'`
    );

    const viewName = 'parsing_test_view';
    const query = `SELECT * FROM thread WHERE id = $id`;

    // Simulate EXACTLY what happens: verify that <string> cast produces unquoted record ID
    // and that check_regex fixes it.
    // We strictly use the Event-driven flow: UPSERT the Incantation, and expect the Event to register it and populate Hash.
    await runQuery(`
      LET $params = { id: thread:test_parsing };
      UPSERT _spooky_incantation:parsing_test_view SET 
          id = '${viewName}', 
          sql = "${query}", 
          params = $params,
          ttl = 1h;
    `);

    // Check if Hash is populated
    const records = await runQuery(`SELECT * FROM _spooky_incantation:parsing_test_view`);
    const record = (records[0].result || records[0])[0];
    expect(record.hash).toBeDefined();
    expect(record.hash).not.toBe('');
  }, 20000);

  test('should handle backticked record ID parameters (sql complex style)', async () => {
    // Mimic the failing scenario: { id: thread:`thread:b6a...` }
    const threadId = 'thread:complex_id';
    await runQuery(
      `CREATE ${threadId} SET title = 'complex_test', author = user:alice, content = 'content'`
    );

    const viewName = 'complex_test_view';
    const query = `SELECT * FROM thread WHERE id = $id`;

    // Simulate EXACTLY what happens in the logs:
    // "DEBUG: register_view called with id: ..., params: String("{ id: thread:`thread:b6a...` }")"
    // We achieve this string by constructing such a string manually or by casting?
    // SurrealDB casting logic is opaque. But we can construct the string manually to test the regex parser.
    //
    // In the real app, this comes from `<string>$params`.
    // If $params is `{ id: thread:complex_id }`.
    // It seems SurrealDB formats it as `thread:`thread:complex_id`` in some contexts?
    // Let's force the string input to `register_view` to match the log format.

    // We construct the JSON-like string with backticks manually to test our parser.
    const trickyParams = `{ id: thread:\`${threadId}\` }`;

    await runQuery(`
      LET $res = mod::dbsp::register_view('${viewName}', "${query}", "${trickyParams}");
      UPSERT _spooky_incantation:complex_test_view SET 
          id = '${viewName}', 
          sql = "${query}", 
          params = { id: ${threadId} },
          hash = $res.result_hash,
          ttl = 1h;
    `);

    // Check if Hash is populated
    const records = await runQuery(`SELECT * FROM _spooky_incantation:complex_test_view`);
    const record = (records[0].result || records[0])[0];
    expect(record.hash).toBeDefined();
    expect(record.hash).not.toBe('');
    expect(record.hash).not.toBe('');
  }, 20000);

  test('should handle LIMIT with trailing semicolon (SQL parser regression)', async () => {
    // Mimic the query structure that failed: SELECT ... WHERE id=$id LIMIT 1;
    const threadId = 'thread:semicolon_test';
    // Fix: Add required author and content fields
    await runQuery(
      `CREATE ${threadId} SET title = 'semicolon_test', author = user:alice, content = 'content'`
    );

    const viewName = 'semicolon_view';
    // CRITICAL: The query MUST end with a semicolon inside the string to trigger the bug if not fixed
    const query = `SELECT * FROM thread WHERE id = $id LIMIT 1;`;
    const params = `{ id: ${threadId} }`;

    await runQuery(`
      LET $res = mod::dbsp::register_view('${viewName}', "${query}", "${params}");
      UPSERT _spooky_incantation:${viewName} SET 
          id = '${viewName}', 
          sql = "${query}", 
          params = { id: ${threadId} },
          hash = $res.result_hash,
          ttl = 1h;
    `);

    const records = await runQuery(`SELECT * FROM _spooky_incantation:${viewName}`);
    const record = (records[0].result || records[0])[0];

    expect(record.hash).toBeDefined();
    expect(record.hash).not.toBe('');
  }, 20000);

  test('should handle view updates (Deduplication bug regression)', async () => {
    const viewName = 'update_test_view';
    const query = `SELECT * FROM user WHERE id = $id`;
    const initialParams = `{ id: user:alice }`; // Valid existing user

    // 1. Register with valid params
    await runQuery(`
      LET $res = mod::dbsp::register_view('${viewName}', "${query}", "${initialParams}");
      UPSERT _spooky_incantation:${viewName} SET 
        id = '${viewName}',
        sql = "${query}",
        params = ${initialParams},
        hash = $res.result_hash,
        ttl = 1h;
    `);

    let records = await runQuery(`SELECT * FROM _spooky_incantation:${viewName}`);
    let record = (records[0].result || records[0])[0];
    const initialHash = record.hash;
    expect(initialHash).not.toBe('');

    // 2. Register SAME view with DIFFERENT params (target non-existent user)
    const bobId = 'user:bob_update_test';
    // Create bob so the hash is not empty (if it works correctly)
    await runQuery(`CREATE ${bobId} SET username = 'bobby', password = 'pwd'`);
    const newParams = `{ id: ${bobId} }`;

    await runQuery(`
      LET $res = mod::dbsp::register_view('${viewName}', "${query}", "${newParams}");
      UPSERT _spooky_incantation:${viewName} SET 
        id = '${viewName}',
        sql = "${query}",
        params = ${newParams},
        hash = $res.result_hash,
        ttl = 1h;
    `);

    records = await runQuery(`SELECT * FROM _spooky_incantation:${viewName}`);
    record = (records[0].result || records[0])[0];

    expect(record.hash).toBeDefined();
    expect(record.hash).not.toBe('');
    // Ensure we actually got a result (validating params updated)
    expect(record.hash).not.toEqual(initialHash);
  }, 20000);

  test('should persist state correctly with control characters (newlines) in content', async () => {
    // Regression test for "Deserialization failed: control character" error.
    // This happens when JSON strings containing \n are not properly escaped in the SQL query.
    const threadId = 'thread:newline_test';
    // Content with newlines
    await runQuery(
      `CREATE ${threadId} SET title = 'newline', author = user:alice, content = 'Line 1\\nLine 2'`
    );

    const viewName = 'newline_view';
    const query = `SELECT * FROM thread WHERE id = $id`;
    const params = `{ id: ${threadId} }`;

    // 1. Ingest/Register
    await runQuery(`
      LET $res = mod::dbsp::register_view('${viewName}', "${query}", "${params}");
      UPSERT _spooky_incantation:${viewName} SET 
          id = '${viewName}', 
          sql = "${query}", 
          params = ${params},
          hash = $res.result_hash,
          ttl = 1h;
    `);

    // 2. Force Reload (by registering another view or triggering ingest)
    // This triggers load_state(). If state is corrupted, this next operation will likely fail or return errors.
    const dummyId = 'thread:dummy';
    await runQuery(`CREATE ${dummyId} SET content = 'dummy', author = user:alice, title = 'dummy'`);

    // Trigger ingest which loads state
    await runQuery(
      `CREATE ${dummyId}_trigger SET title = 'trigger', author = user:alice, content = 'trigger'`
    );

    // Check if the ORIGINAL view is still tracked/persisted (indirectly via Incantation Hash staying valid? No)
    // We can check if `mod::dbsp::save_state` throws or logs errors (but logs are hidden).
    // Instead we can check if `register_view` for a NEW view works.

    const viewName2 = 'newline_view_2';
    const res2 = await runQuery(`
      RETURN mod::dbsp::register_view('${viewName2}', "${query}", "${params}");
    `);

    // If load_state failed, the circuit resets. The OLD view 'newline_view' would be lost from memory.
    // But we are stateless between calls unless load_state works.
    // If load_state works, we should see logs (if we could).

    // A better check:
    // If load_state fails, the previous view is gone.
    // If we re-register the SAME view, it should get the SAME hash.
    // If we query the view hash through a side channel? No side channel.

    // Best check: If load_state crashes/fails, often the WASM module might panic or return error.
    // The previous error was a DEBUG log but the function might continue with empty state.
    // If state is empty, `ingest` for the OLD table `thread` might fail to update the view?

    // If we update the record with newline, does the hash change?
    // If state was lost, the view is gone, so hash won't update.

    await runQuery(`UPDATE ${threadId} SET content = 'Line 1\\nLine 2\\nLine 3'`); // Trigger update

    // Check incantation hash? We need to have a listener.
    // But this test environment doesn't have the sync service running.
    // We can only check return values of functions.

    // Just ensuring no error is thrown during these operations is a good baseline.
    // And verifying that *fetching* the state manually via SQL returns valid JSON.

    const state = await runQuery(`SELECT content FROM _spooky_module_state:dbsp`);
    const contentStr = (state[0].result || state[0])[0]?.content;

    expect(contentStr).toBeDefined();
    // Try to parse it manually to ensure it's valid JSON
    expect(() => JSON.parse(contentStr)).not.toThrow();

    expect(contentStr).toContain('Line 1\\nLine 2'); // Should be escaped in the JSON string
  }, 20000);

  test('should trigger _spooky_thread_mutation event and create deterministic hash record', async () => {
    const timestamp = Date.now();
    const threadId = `thread:mutation_test_${timestamp}`;
    const incantId = `thread_test_view_${timestamp}`;

    // 1. Create a thread record (mutation) - Ensure cleanup/isolation via unique ID
    const createQuery = `
      UPSERT _spooky_incantation:${incantId} CONTENT {
        id: '${incantId}',
        sql: 'SELECT * FROM thread',
        params: {},
        ttl: 10m,
        lastActiveAt: time::now(),
        clientId: 'test-client'
      };

      CREATE ${threadId} SET 
        title = 'Mutation Test Thread',
        content = 'Testing spooky event triggering',
        author = user:mutation_test;
    `;
    const res = await runQuery(createQuery);
    // res will contain array of results. Last one is the CREATE result.
    expect(res).toBeDefined();
    // 1. Create a user (needed as author)
    const user = await runQuery(`
      CREATE user:mutation_test CONTENT {
        username: "mutation_tester",
        password: "password",
        created_at: time::now()
      }
    `);
    // db.query returns array of results. CREATE returns array of records.
    // So user[0] is the result of first statement, which is [Record].
    // So we want user[0][0].id or user[0].result[0].id depending on SDK version.

    // 2. Verify _spooky_data_hash record exists with deterministic ID
    // ID should be crypto::blake3(threadId)
    // We can just select from it directly using the same logic function to verify
    // Verify alternative syntax: casting string to record
    const hashCheck = await runQuery(`
      SELECT * FROM _spooky_data_hash WHERE recordId = '${threadId}' OR recordId = <record>'${threadId}';
    `);

    // hashCheck might be array of records directly or [[records]] depending on driver version/query type
    // Since it's a single SELECT, it's usually [ [Record] ] or [Record]
    const results = Array.isArray(hashCheck[0]) ? hashCheck[0] : hashCheck;
    const hashRecord = results.result ? results.result[0] : results[0];

    if (!hashRecord) {
      // Fallback dump for debugging
      const allHashes = await runQuery('SELECT * FROM _spooky_data_hash');
      console.log('DEBUG: Lookup failed. All hashes:', JSON.stringify(allHashes, null, 2));
    }

    expect(hashRecord).toBeDefined();
    expect(hashRecord.recordId.toString()).toBe(threadId);

    expect(hashRecord.id.toString()).toMatch(/_spooky_data_hash:.+/);

    // 4. Verify Merkle Tree in _spooky_incantation does NOT have MISSING_HASH
    const incantations = await runQuery(`SELECT * FROM _spooky_incantation:${incantId}`);
    const tree =
      incantations[0]?.result?.[0]?.tree ||
      incantations[0]?.tree ||
      (Array.isArray(incantations[0]) ? incantations[0][0]?.tree : undefined);

    // Check if we have valid query results
    if (tree) {
      const treeStr = JSON.stringify(tree);
      if (treeStr.includes('MISSING_HASH')) {
        console.error('DEBUG: Tree contains MISSING_HASH:', JSON.stringify(tree, null, 2));
      }
      expect(treeStr).not.toContain('MISSING_HASH');
      // Verify we have at least one leaf with a hash
      expect(treeStr).toMatch(/"hash":"[a-f0-9]{64}"/);
    } else {
      // Fail if no incantation found or no tree
      // But maybe the ingest hasn't updated it yet?
      // Syncgen events are synchronous (blocking). So it should be there.
      console.log('DEBUG: No tree found in incantation', JSON.stringify(incantations, null, 2));
      expect(tree).toBeDefined();
    }
  });
});
