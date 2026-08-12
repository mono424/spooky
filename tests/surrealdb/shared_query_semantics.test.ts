import { createTestDb, clearTestDb, TEST_DB_CONFIG, getTestDbPort } from './setup';
import { Surreal } from 'surrealdb';

/**
 * Phase 0 of the shared-query work: pin the SurrealDB behaviours the design
 * rests on, BEFORE anything depends on them.
 *
 * Each assertion here corresponds to a design decision that would be wrong if
 * the engine disagreed. They are deliberately about the engine, not about our
 * code, so they double as an upgrade tripwire: if a SurrealDB bump changes any
 * of these, the shared-query design needs revisiting rather than debugging.
 */
describe('shared-query SurrealDB semantics', () => {
  let root: Surreal;

  const q = async (db: Surreal, sql: string, vars?: Record<string, unknown>) =>
    await (db.query(sql, vars) as any).collect();

  /** A second connection authenticated as a record user. */
  async function connectAs(email: string): Promise<Surreal> {
    const db = new Surreal();
    await db.connect(`ws://127.0.0.1:${getTestDbPort()}/rpc`, {
      namespace: TEST_DB_CONFIG.namespace,
      database: TEST_DB_CONFIG.database,
    });
    await db.signin({
      namespace: TEST_DB_CONFIG.namespace,
      database: TEST_DB_CONFIG.database,
      access: 'user_access',
      variables: { email },
    } as any);
    return db;
  }

  beforeAll(async () => {
    root = await createTestDb();

    // Minimal record-level auth so permission rules can actually be exercised.
    // Root bypasses PERMISSIONS entirely, so none of this is testable as root.
    await q(
      root,
      `
      DEFINE TABLE OVERWRITE person SCHEMALESS PERMISSIONS FULL;
      DEFINE ACCESS OVERWRITE user_access ON DATABASE TYPE RECORD
        SIGNIN ( SELECT * FROM person WHERE email = $email )
        DURATION FOR SESSION 1h, FOR TOKEN 1h;
      DELETE person;
      CREATE person:alice SET email = 'alice@test';
      CREATE person:bob   SET email = 'bob@test';
      `
    );
  }, 180_000);

  afterAll(async () => {
    if (root) {
      await clearTestDb(root);
      await root.close();
    }
  });

  /**
   * The single most load-bearing claim in the design. `_00_query` grants only
   * `select, create, update`, and we concluded that the client's `DELETE $id`
   * has therefore always been a silent no-op — which is why the TTL sweep is
   * the only teardown that ever worked, and why granting `delete` is safe to
   * add rather than a behaviour change.
   *
   * If this test fails, eager teardown DID work and the plan's teardown
   * reasoning has to be redone.
   */
  test('permission kinds that are not listed default to NONE', async () => {
    await q(
      root,
      `
      DEFINE TABLE OVERWRITE gated SCHEMALESS
        PERMISSIONS FOR select, create, update WHERE owner = $auth.id;
      DELETE gated;
      CREATE gated:one SET owner = person:alice, n = 1;
      `
    );

    const alice = await connectAs('alice@test');
    try {
      // She can read her own row: the listed kinds work as expected.
      const read = await q(alice, `SELECT VALUE n FROM gated:one`);
      expect(read[0]).toEqual([1]);

      // DELETE is unlisted. It must affect nothing AND must not throw, which
      // is exactly why the client never noticed.
      await q(alice, `DELETE gated:one`);

      const stillThere = await q(root, `SELECT VALUE n FROM gated:one`);
      expect(stillThere[0]).toEqual([1]);
    } finally {
      await alice.close();
    }
  });

  /**
   * The new `_00_query` rule casts `$auth.id`. An anonymous session has
   * `$auth = NONE`, and we guard the cast with `$auth.id != NONE`. Confirm the
   * guard is actually required (i.e. that the bare cast is a problem) and that
   * the guarded form evaluates cleanly for both anon and authenticated.
   */
  test('the auth-scoped rule holds for anon and authenticated sessions', async () => {
    await q(
      root,
      `
      DEFINE TABLE OVERWRITE scoped SCHEMALESS
        PERMISSIONS FOR select
          WHERE auth_id = 'anon'
             OR ($auth.id != NONE AND auth_id = <string>$auth.id);
      DELETE scoped;
      CREATE scoped:pub  SET auth_id = 'anon';
      CREATE scoped:mine SET auth_id = 'person:alice';
      CREATE scoped:hers SET auth_id = 'person:bob';
      `
    );

    // Anonymous (no signin): sees only the anon row, and does not error on the
    // cast despite $auth being NONE.
    const anon = new Surreal();
    await anon.connect(`ws://127.0.0.1:${getTestDbPort()}/rpc`, {
      namespace: TEST_DB_CONFIG.namespace,
      database: TEST_DB_CONFIG.database,
    });
    try {
      const seen = await q(anon, `SELECT VALUE id FROM scoped ORDER BY id`);
      expect(seen[0].map(String)).toEqual(['scoped:pub']);
    } finally {
      await anon.close();
    }

    // Authenticated: sees the anon row plus her own, never another user's.
    const alice = await connectAs('alice@test');
    try {
      const seen = await q(alice, `SELECT VALUE id FROM scoped ORDER BY id`);
      expect(seen[0].map(String).sort()).toEqual(['scoped:mine', 'scoped:pub']);
    } finally {
      await alice.close();
    }
  });

  /**
   * The heartbeat and the register append both prune stale subscribers in the
   * SAME statement that adds one, to avoid a read-modify-write round trip.
   * That requires bare field references (`ttl`, `subscribers`) to resolve
   * against the current document inside `UPDATE ... SET`, and
   * `array::filter` closures to work over a field of a SCHEMALESS table.
   */
  test('a single UPDATE can prune by the row own ttl and append atomically', async () => {
    await q(
      root,
      `
      DEFINE TABLE OVERWRITE q SCHEMALESS;
      -- No \`subscribers[*] ... FLEXIBLE\`: FLEXIBLE is rejected outright on a
      -- SCHEMALESS table, and _00_query is SCHEMALESS. A bare array<object>
      -- is what we actually get to use, so pin that it accepts arbitrary
      -- object keys here rather than discovering it in production.
      DEFINE FIELD OVERWRITE subscribers ON TABLE q TYPE array<object> DEFAULT [];
      DELETE q;
      CREATE q:one SET ttl = 1h, subscribers = [
        { id: 'fresh', seenAt: time::now() },
        { id: 'stale', seenAt: time::now() - 2h },
        { id: 'mine',  seenAt: time::now() - 2h }
      ];
      `
    );

    // Exactly the shape fn::query::heartbeat will use.
    await q(
      root,
      `
      UPDATE q:one SET
        lastActiveAt = time::now(),
        subscribers = array::append(
          array::filter(subscribers ?? [], |$s|
            <string>$s.id != $sid AND <datetime>$s.seenAt + ttl > time::now()
          ),
          { id: $sid, seenAt: time::now() }
        );
      `,
      { sid: 'mine' }
    );

    const ids = await q(root, `SELECT VALUE subscribers.map(|$s| $s.id) FROM ONLY q:one`);
    // 'stale' pruned by ttl; 'mine' de-duplicated then re-appended; 'fresh' kept.
    expect(ids[0].sort()).toEqual(['fresh', 'mine']);
  });

  /**
   * Phase 1: the REAL DDL and functions, loaded verbatim from the shipped
   * .surql files rather than retyped here, so this fails if the files drift.
   *
   * This is the two-tab scenario that motivated the change, at the DB layer:
   * two sessions of one user share a row, both can read it, both can
   * heartbeat, and the first to leave does NOT tear it down for the second.
   */
  describe('shipped _00_query schema and functions', () => {
    const fs = require('fs');
    const path = require('path');
    const read = (f: string) =>
      fs.readFileSync(path.resolve(__dirname, '../../apps/cli/src', f), 'utf8');

    const ID = '_00_query:shared';

    beforeAll(async () => {
      // Only the _00_query block; the rest of the file defines tables this
      // test does not need.
      const meta = read('meta_tables_remote.surql');
      const start = meta.indexOf('DEFINE TABLE OVERWRITE _00_query');
      const end = meta.indexOf('-- SPOOKY LIST REF');
      await q(root, meta.slice(start, end));
      await q(root, read('functions_remote.surql'));
      // This layer tests the schema and the functions, not the SSP round trip.
      // The cleanup event HTTP-POSTs /view/unregister, and there is no SSP in
      // the container, so a DELETE would fail on a connection error that says
      // nothing about the rules under test.
      await q(root, `REMOVE EVENT IF EXISTS _00_dbsp_cleanup ON TABLE _00_query;`);
    }, 60_000);

    beforeEach(async () => {
      await q(
        root,
        `DELETE _00_query;
         CREATE ${ID} SET auth_id = 'person:alice', ttl = 1h, rowCount = 7,
                          lastActiveAt = time::now() - 30m, subscribers = [];`
      );
    });

    test('both sessions of one user can read the shared row', async () => {
      const a = await connectAs('alice@test');
      const b = await connectAs('alice@test');
      try {
        // The probe that decides "empty" vs "still flushing". Under the old
        // session-scoped rule this returned NONE for whichever tab lost.
        for (const db of [a, b]) {
          const seen = await q(db, `SELECT VALUE rowCount FROM ONLY ${ID}`);
          expect(seen[0]).toBe(7);
        }
      } finally {
        await a.close();
        await b.close();
      }
    });

    test('another user cannot read it', async () => {
      const bob = await connectAs('bob@test');
      try {
        const seen = await q(bob, `SELECT VALUE rowCount FROM ONLY ${ID}`);
        expect(seen[0] ?? null).toBeNull();
      } finally {
        await bob.close();
      }
    });

    test('either session can heartbeat, and each registers itself', async () => {
      const a = await connectAs('alice@test');
      const b = await connectAs('alice@test');
      try {
        const before = (await q(root, `SELECT VALUE lastActiveAt FROM ONLY ${ID}`))[0];

        await q(a, `RETURN fn::query::heartbeat(${ID})`);
        await q(b, `RETURN fn::query::heartbeat(${ID})`);

        const after = (await q(root, `SELECT VALUE lastActiveAt FROM ONLY ${ID}`))[0];
        expect(new Date(after).getTime()).toBeGreaterThan(new Date(before).getTime());

        // Two distinct sessions are now recorded.
        const subs = (await q(root, `SELECT VALUE array::len(subscribers) FROM ONLY ${ID}`))[0];
        expect(subs).toBe(2);
      } finally {
        await a.close();
        await b.close();
      }
    });

    test('a heartbeat re-stamps rather than duplicating the same session', async () => {
      const a = await connectAs('alice@test');
      try {
        await q(a, `RETURN fn::query::heartbeat(${ID})`);
        await q(a, `RETURN fn::query::heartbeat(${ID})`);
        const subs = (await q(root, `SELECT VALUE array::len(subscribers) FROM ONLY ${ID}`))[0];
        expect(subs).toBe(1);
      } finally {
        await a.close();
      }
    });

    /** THE regression this whole change exists to prevent. */
    test('the first session leaving does not tear the row down', async () => {
      const a = await connectAs('alice@test');
      const b = await connectAs('alice@test');
      try {
        await q(a, `RETURN fn::query::heartbeat(${ID})`);
        await q(b, `RETURN fn::query::heartbeat(${ID})`);

        const first = await q(a, `RETURN fn::query::unsubscribe(${ID})`);
        expect(first[first.length - 1]).toMatchObject({ released: false, remaining: 1 });

        // B is still watching, so the row — and therefore the view and every
        // edge — must survive.
        const alive = (await q(root, `SELECT VALUE rowCount FROM ONLY ${ID}`))[0];
        expect(alive).toBe(7);
        expect((await q(b, `SELECT VALUE rowCount FROM ONLY ${ID}`))[0]).toBe(7);

        const last = await q(b, `RETURN fn::query::unsubscribe(${ID})`);
        expect(last[last.length - 1]).toMatchObject({ released: true, remaining: 0 });

        const gone = await q(root, `SELECT VALUE id FROM _00_query`);
        expect(gone[0].length).toBe(0);
      } finally {
        await a.close();
        await b.close();
      }
    });

    test('a crashed session does not pin the row: TTL still expires it', async () => {
      const a = await connectAs('alice@test');
      try {
        await q(a, `RETURN fn::query::heartbeat(${ID})`);
      } finally {
        await a.close(); // never unsubscribes
      }
      // Age the row past its TTL the way the sweep sees it.
      await q(root, `UPDATE ${ID} SET lastActiveAt = time::now() - 2h`);
      const expired = await q(
        root,
        `SELECT VALUE id FROM _00_query WHERE lastActiveAt + ttl < time::now()`
      );
      // The sweep's predicate still selects it despite a non-empty subscriber set.
      expect(expired[0].length).toBe(1);
      const subs = (await q(root, `SELECT VALUE array::len(subscribers) FROM ONLY ${ID}`))[0];
      expect(subs).toBe(1);
    });

    test('an anon row is readable without auth', async () => {
      await q(root, `CREATE _00_query:pub SET auth_id = 'anon', ttl = 1h, rowCount = 3;`);
      const anon = new Surreal();
      await anon.connect(`ws://127.0.0.1:${getTestDbPort()}/rpc`, {
        namespace: TEST_DB_CONFIG.namespace,
        database: TEST_DB_CONFIG.database,
      });
      try {
        expect((await q(anon, `SELECT VALUE rowCount FROM ONLY _00_query:pub`))[0]).toBe(3);
        // ...but not another user's row.
        expect((await q(anon, `SELECT VALUE rowCount FROM ONLY ${ID}`))[0] ?? null).toBeNull();
      } finally {
        await anon.close();
      }
    });
  });

  /**
   * Release is only correct if removing the last subscriber and deleting the
   * row happen against a consistent read. This exercises the unsubscribe
   * shape end to end: not-last leaves the row, last removes it.
   */
  test('unsubscribe removes the row only when the last subscriber leaves', async () => {
    await q(
      root,
      `
      DELETE q;
      CREATE q:two SET ttl = 1h, subscribers = [
        { id: 'a', seenAt: time::now() },
        { id: 'b', seenAt: time::now() }
      ];
      `
    );

    const unsubscribe = async (sid: string) =>
      await q(
        root,
        `
        LET $row  = (SELECT subscribers, ttl FROM ONLY q:two);
        LET $rest = IF $row = NONE { [] } ELSE {
          array::filter($row.subscribers ?? [], |$s|
            <string>$s.id != $sid AND <datetime>$s.seenAt + $row.ttl > time::now()
          )
        };
        IF $row != NONE AND array::len($rest) = 0 { DELETE q:two } ELSE { UPDATE q:two SET subscribers = $rest };
        RETURN array::len($rest);
        `,
        { sid }
      );

    const afterA = await unsubscribe('a');
    expect(afterA[afterA.length - 1]).toBe(1);
    const survives = await q(root, `SELECT VALUE id FROM q:two`);
    expect(survives[0].length).toBe(1);

    const afterB = await unsubscribe('b');
    expect(afterB[afterB.length - 1]).toBe(0);
    const gone = await q(root, `SELECT VALUE id FROM q:two`);
    expect(gone[0].length).toBe(0);
  });
});
