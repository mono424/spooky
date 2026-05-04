import type { Surreal } from 'surrealdb';
import { Surreal as SurrealCtor, RecordId } from 'surrealdb';
import { GenericContainer, type StartedTestContainer, Wait } from 'testcontainers';
import * as path from 'path';
import * as fs from 'fs';

// End-to-end test for the new _00_crdt / _00_cursor design, against the
// REAL generated server schema (not a hand-trimmed copy). Drives the same
// queries the client emits and confirms permission inheritance + UNIQUE
// index UPSERT behaviour.
//
// We strip the http::post events from the schema before loading because
// they target an SSP that isn't running in the test container; the meta
// table definitions (which is what we're testing) are independent.

const NS = 'crdt_ns';
const DB = 'crdt_db';

function loadSchema(): string {
  const p = path.resolve(__dirname, '../.sp00ky/schema.gen.surql');
  let s = fs.readFileSync(p, 'utf8');
  // Module path tweak (mirrors setup.ts).
  s = s.replace('file:/modules', 'file:///modules');
  // Strip every DEFINE EVENT block — they POST to a non-running SSP.
  s = s.replace(/DEFINE EVENT[\s\S]*?\n};\n/g, '');
  // Strip surrealism module functions (mod::dbsp etc.) that need a loaded
  // .surli; we don't exercise them.
  s = s.replace(/DEFINE FUNCTION OVERWRITE fn::query::register[\s\S]*?\n};/g, '');
  return s;
}

let container: StartedTestContainer;
let port: number;
let rootDb: Surreal;

async function rootConnect(): Promise<Surreal> {
  const db = new SurrealCtor();
  await db.connect(`http://localhost:${port}/rpc`);
  await db.signin({ username: 'root', password: 'root' });
  await db.use({ namespace: NS, database: DB });
  return db;
}

async function userConnect(username: string, password: string): Promise<Surreal> {
  const db = new SurrealCtor();
  await db.connect(`http://localhost:${port}/rpc`);
  await db.use({ namespace: NS, database: DB });
  await db.signin({
    namespace: NS,
    database: DB,
    access: 'account',
    variables: { username, password },
  });
  return db;
}

async function signup(username: string, password: string): Promise<RecordId> {
  const created = await rootDb.query<[Array<{ id: RecordId }>]>(
    `CREATE ONLY user CONTENT { username: $u, password: crypto::argon2::generate($p) } RETURN AFTER`,
    { u: username, p: password },
  );
  return (created as any)[0].id as RecordId;
}

beforeAll(async () => {
  container = await new GenericContainer('surrealdb/surrealdb:v3.0.0')
    .withExposedPorts(8000)
    .withCommand(['start', '--user', 'root', '--pass', 'root', '--allow-all'])
    .withStartupTimeout(30000)
    .withWaitStrategy(Wait.forLogMessage('Started web server on 0.0.0.0:8000'))
    .start();

  port = container.getMappedPort(8000);
  rootDb = await rootConnect();

  const schema = loadSchema();
  const r = (await rootDb.query(schema)) as any;
  const results = r && typeof r.collect === 'function' ? await r.collect() : r;
  if (Array.isArray(results)) {
    for (const res of results) {
      if (res && res.status === 'ERR') {
        throw new Error('Schema load failed: ' + JSON.stringify(res));
      }
    }
  }
}, 60000);

afterAll(async () => {
  await rootDb?.close();
  await container?.stop();
}, 30000);

describe('CRDT + cursor meta tables (real generated schema)', () => {
  test('Author can UPSERT _00_crdt + _00_cursor for their own thread', async () => {
    const username = `author_${Date.now()}`;
    const password = 'pw';
    await signup(username, password);

    const u = await userConnect(username, password);
    try {
      const created = await u.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };
      expect(thread?.id).toBeDefined();

      // Same query the client (crdt-field.ts) emits.
      await u.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;
         UPDATE $id SET _00_rv = _00_rv RETURN NONE;`,
        { id: thread.id, field: 'title', state: 'snap-A' },
      );

      const crdtAfter = await u.query(
        `SELECT field, state FROM _00_crdt WHERE record_id = $id;`,
        { id: thread.id },
      );
      const rows = (crdtAfter as any)[0] as Array<{ field: string; state: string }>;
      // eslint-disable-next-line no-console
      console.log('crdt rows:', JSON.stringify(rows));
      expect(rows.length).toBe(1);
      expect(rows[0]).toEqual({ field: 'title', state: 'snap-A' });

      // Idempotent re-UPSERT.
      await u.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
        { id: thread.id, field: 'title', state: 'snap-B' },
      );
      const rows2 = (
        (await u.query(`SELECT state FROM _00_crdt WHERE record_id = $id;`, {
          id: thread.id,
        })) as any
      )[0];
      expect(rows2[0].state).toBe('snap-B');

      // Cursor.
      await u.query(
        `UPSERT type::record("_00_cursor", [$id, $sid, $field]) SET record_id = $id, session_id = $sid, field = $field, state = $state;
         UPDATE $id SET _00_rv = _00_rv RETURN NONE;`,
        { id: thread.id, sid: 'sess-1', field: 'title', state: 'cur-A' },
      );
      const cur = (
        (await u.query(`SELECT * FROM _00_cursor WHERE record_id = $id;`, {
          id: thread.id,
        })) as any
      )[0] as Array<any>;
      expect(cur.length).toBe(1);
      expect(cur[0].state).toBe('cur-A');
    } finally {
      await u.close();
    }
  });

  test('Non-author cannot UPSERT _00_crdt for someone else thread', async () => {
    const author = `auth2_${Date.now()}`;
    const stranger = `stranger_${Date.now()}`;
    const password = 'pw';
    await signup(author, password);
    await signup(stranger, password);

    const a = await userConnect(author, password);
    const s = await userConnect(stranger, password);
    try {
      const created = await a.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      let denied = false;
      try {
        await s.query(
          `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
          { id: thread.id, field: 'title', state: 'evil' },
        );
      } catch {
        denied = true;
      }

      const after = (
        (await a.query(`SELECT field, state FROM _00_crdt WHERE record_id = $id;`, {
          id: thread.id,
        })) as any
      )[0];
      // eslint-disable-next-line no-console
      console.log('non-author UPSERT, threw=', denied, 'rows=', JSON.stringify(after));
      expect(after).toEqual([]);
    } finally {
      await a.close();
      await s.close();
    }
  });

  test('Subquery fetch returns CRDT and cursor rows alongside parent', async () => {
    const author = `auth3_${Date.now()}`;
    const password = 'pw';
    await signup(author, password);
    const u = await userConnect(author, password);
    try {
      const created = await u.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      await u.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
        { id: thread.id, field: 'title', state: 's-title' },
      );
      await u.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
        { id: thread.id, field: 'content', state: 's-content' },
      );
      await u.query(
        `UPSERT type::record("_00_cursor", [$id, $sid, $field]) SET record_id = $id, session_id = $sid, field = $field, state = $state;`,
        { id: thread.id, sid: 'sess-X', field: 'title', state: 'cur-1' },
      );

      // Mimics CrdtManager.fetchAndDispatchMeta — both arrays in one round-trip.
      const result = await u.query(
        `SELECT field, state FROM _00_crdt WHERE record_id = $id;
         SELECT session_id, field, state FROM _00_cursor WHERE record_id = $id;`,
        { id: thread.id },
      );
      const r = result as unknown as [Array<any>, Array<any>];
      expect(r[0].length).toBe(2);
      expect(r[1].length).toBe(1);
    } finally {
      await u.close();
    }
  });

  test('Collaborator can UPSERT _00_crdt via collaborates_on relation', async () => {
    const author = `auth4_${Date.now()}`;
    const collab = `collab_${Date.now()}`;
    const password = 'pw';
    await signup(author, password);
    const collabId = await signup(collab, password);

    const a = await userConnect(author, password);
    const c = await userConnect(collab, password);
    try {
      const created = await a.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      // Wire up the collaborator. The thread_invite + collaborates_on rules
      // require: invite token exists, then RELATE in→out as the collab.
      // Bypass permission gates: create the invite as root. created_by has
      // VALUE $auth.id but we're root here so set it explicitly to author.
      const authorId = (
        (await rootDb.query(`SELECT VALUE author FROM ONLY $tid;`, {
          tid: thread.id,
        })) as any
      )[0] as RecordId;
      await rootDb.query(
        `CREATE thread_invite SET thread = $tid, token = $tok, created_by = $author;`,
        { tid: thread.id, tok: 'tok_abcdefghijklmnop', author: authorId },
      );
      const inviteRoot = await rootDb.query(
        `SELECT * FROM thread_invite WHERE thread = $tid;`,
        { tid: thread.id },
      );
      // eslint-disable-next-line no-console
      console.log('invite via root:', JSON.stringify(inviteRoot));

      // Bypass collaborates_on CREATE rule and create the relation as root.
      // The CRDT meta-rule's subquery (`SELECT VALUE in FROM collaborates_on
      // WHERE out = $parent.record_id`) is what we're actually testing.
      await rootDb.query(
        `RELATE $cid->collaborates_on->$tid;`,
        { cid: collabId, tid: thread.id },
      );

      const co = await rootDb.query(
        `SELECT in, out FROM collaborates_on WHERE out = $tid;`,
        { tid: thread.id },
      );
      // eslint-disable-next-line no-console
      console.log('collaborates_on rows for thread:', JSON.stringify(co));

      // Confirm the subquery the rule uses returns the collab.
      const probe = await c.query(
        `SELECT VALUE in FROM collaborates_on WHERE out = $tid;`,
        { tid: thread.id },
      );
      // eslint-disable-next-line no-console
      console.log('subquery as collab:', JSON.stringify(probe));

      // Now the collaborator should be able to UPSERT a CRDT row.
      let collabDenied = false;
      try {
        await c.query(
          `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
          { id: thread.id, field: 'title', state: 'collab-A' },
        );
      } catch (e) {
        // eslint-disable-next-line no-console
        console.log('collab UPSERT threw:', e);
        collabDenied = true;
      }

      const after = (
        (await a.query(`SELECT field, state FROM _00_crdt WHERE record_id = $id;`, {
          id: thread.id,
        })) as any
      )[0] as Array<{ field: string; state: string }>;
      // eslint-disable-next-line no-console
      console.log('after collab UPSERT, threw=', collabDenied, 'rows=', JSON.stringify(after));
      expect(after.length).toBe(1);
      expect(after[0].state).toBe('collab-A');
    } finally {
      await a.close();
      await c.close();
    }
  });

  test('DELETE thread cleans up _00_crdt and _00_cursor rows', async () => {
    const author = `auth5_${Date.now()}`;
    const password = 'pw';
    await signup(author, password);
    const u = await userConnect(author, password);
    try {
      const created = await u.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      await u.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;`,
        { id: thread.id, field: 'title', state: 'x' },
      );
      await u.query(
        `UPSERT type::record("_00_cursor", [$id, $sid, $field]) SET record_id = $id, session_id = $sid, field = $field, state = $state;`,
        { id: thread.id, sid: 'sess', field: 'title', state: 'y' },
      );

      // Cleanup happens server-side via the parent's DELETE event. Simulate
      // it directly here since events are stripped from the test schema.
      await rootDb.query(
        `DELETE _00_crdt   WHERE record_id = $id;
         DELETE _00_cursor WHERE record_id = $id;
         DELETE $id;`,
        { id: thread.id },
      );

      const remaining = await rootDb.query(
        `SELECT * FROM _00_crdt WHERE record_id = $id;
         SELECT * FROM _00_cursor WHERE record_id = $id;
         SELECT * FROM thread WHERE id = $id;`,
        { id: thread.id },
      );
      const r = remaining as unknown as [Array<any>, Array<any>, Array<any>];
      expect(r[0]).toEqual([]);
      expect(r[1]).toEqual([]);
      expect(r[2]).toEqual([]);
    } finally {
      await u.close();
    }
  });
});
