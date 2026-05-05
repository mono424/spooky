import type { Surreal } from 'surrealdb';
import { Surreal as SurrealCtor, RecordId } from 'surrealdb';
import { GenericContainer, type StartedTestContainer, Wait } from 'testcontainers';
import * as path from 'path';
import * as fs from 'fs';

// End-to-end test for the inline CRDT design.
//
// `thread.title` and `thread.content` are annotated `@crdt text` in
// example/schema/src/schema.surql. With the consolidation, those fields
// hold the LoroDoc snapshot directly on the parent row — no sidecar
// `_00_crdt` table — so the editor writes via plain UPDATE on the parent
// and sync-down naturally carries the snapshot.
//
// The schema also defines `_00_version` and assorted other meta tables;
// we strip event handlers (which POST to a non-running SSP) and
// surrealism module functions (which need a loaded `.surli`) before
// loading.

const NS = 'crdt_ns';
const DB = 'crdt_db';

function loadSchema(): string {
  const p = path.resolve(__dirname, '../.sp00ky/schema.gen.surql');
  let s = fs.readFileSync(p, 'utf8');
  s = s.replace('file:/modules', 'file:///modules');
  s = s.replace(/DEFINE EVENT[\s\S]*?\n};\n/g, '');
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

describe('Inline CRDT field on parent record', () => {
  test('Author can UPDATE their own thread.title with a snapshot and read it back', async () => {
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

      // Same shape the client (crdt-field.ts) writes for a `@crdt`-only
      // field: snapshot stored directly in the field.
      await u.query(`UPDATE $id SET title = $state RETURN NONE;`, {
        id: thread.id,
        state: 'snap-A',
      });

      const got = (
        (await u.query(`SELECT VALUE title FROM ONLY $id;`, { id: thread.id })) as any
      )[0];
      expect(got).toBe('snap-A');

      // Re-write — UPDATE is idempotent in the new design (no UPSERT/UNIQUE
      // dance because there's no sidecar row to deduplicate).
      await u.query(`UPDATE $id SET title = $state RETURN NONE;`, {
        id: thread.id,
        state: 'snap-B',
      });
      const got2 = (
        (await u.query(`SELECT VALUE title FROM ONLY $id;`, { id: thread.id })) as any
      )[0];
      expect(got2).toBe('snap-B');
    } finally {
      await u.close();
    }
  });

  test('Non-author cannot UPDATE thread.title on someone else\'s thread', async () => {
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

      // Stranger's UPDATE must not change the field — SurrealDB's parent
      // table UPDATE permission gates the snapshot the same way it
      // gates any other column.
      try {
        await s.query(`UPDATE $id SET title = $state RETURN NONE;`, {
          id: thread.id,
          state: 'evil',
        });
      } catch {
        // Some setups throw here; others silently no-op. We assert on the
        // resulting value below either way.
      }

      const after = (
        (await a.query(`SELECT VALUE title FROM ONLY $id;`, { id: thread.id })) as any
      )[0];
      expect(after).toBe('init');
    } finally {
      await a.close();
      await s.close();
    }
  });

  test('Sync-down: SELECT * FROM thread WHERE id = $id returns the inline snapshot', async () => {
    // The whole point of consolidation: the engine.ts sync-down query
    // already returns CRDT state without any extra fetch.
    const username = `sync_${Date.now()}`;
    const password = 'pw';
    await signup(username, password);
    const u = await userConnect(username, password);
    try {
      const created = await u.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      await u.query(`UPDATE $id SET title = $state RETURN NONE;`, {
        id: thread.id,
        state: 'snap-sync',
      });

      const row = (
        (await u.query(`SELECT * FROM ONLY $id;`, { id: thread.id })) as any
      )[0];
      expect(row).toBeDefined();
      expect(row.title).toBe('snap-sync');
    } finally {
      await u.close();
    }
  });

  test('DELETE thread removes the inline snapshot along with the row', async () => {
    const username = `del_${Date.now()}`;
    const password = 'pw';
    await signup(username, password);
    const u = await userConnect(username, password);
    try {
      const created = await u.query(
        `CREATE ONLY thread CONTENT { title: 'init', content: 'init', author: $auth.id } RETURN AFTER`,
      );
      const head = (created as any)[0];
      const thread = (Array.isArray(head) ? head[0] : head) as { id: RecordId };

      await u.query(`UPDATE $id SET title = $state RETURN NONE;`, {
        id: thread.id,
        state: 'will-go',
      });
      await rootDb.query(`DELETE $id;`, { id: thread.id });

      const remaining = (
        (await rootDb.query(`SELECT * FROM thread WHERE id = $id;`, { id: thread.id })) as any
      )[0];
      expect(remaining).toEqual([]);
    } finally {
      await u.close();
    }
  });
});
