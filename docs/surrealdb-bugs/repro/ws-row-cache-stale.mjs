// Minimal reproduction for the WS row-cache staleness bug.
//
// Goal: isolate which combination of variables triggers
// SurrealDB returning stale row content to a record-authenticated
// session after another session UPDATEs the row.
//
// Variables tested (one per test case below):
//   - Transport: ws:// vs http://
//   - Auth: root (Basic) vs record-user (signin)
//   - Prior fetch on B: yes / no (does priming the row "lock" its content?)
//   - LIVE subscription on B: yes / no (does LIVE prime a snapshot?)
//   - Fresh signin per fetch on B: yes / no (does reusing a session reuse a cache?)
//
// Run:
//   node docs/surrealdb-bugs/repro/ws-row-cache-stale.mjs
//
// Requires the example dev stack to be up (localhost:8666 with the
// example app schema applied).

// Inject the `ws` package as a global WebSocket so the SurrealDB SDK
// can use the WS engine. Node 22's built-in WebSocket implementation
// is experimental and hangs on the SurrealDB SDK's handshake.
import WebSocket from '/Users/khadim/dev/spooky/node_modules/.pnpm/ws@8.19.0/node_modules/ws/index.js';
globalThis.WebSocket = WebSocket;

import { Surreal } from 'surrealdb';
import { randomBytes } from 'node:crypto';

// ── helpers ─────────────────────────────────────────────────────

const WS = 'ws://localhost:8666/rpc';
const HTTP = 'http://localhost:8666/rpc';
const NS = 'main';
const DB = 'example';
const ROOT_BASIC = 'Basic ' + Buffer.from('root:root').toString('base64');

const rand = (prefix) => `${prefix}_${randomBytes(4).toString('hex')}`;

/** Direct /sql POST with root credentials — ground truth. */
async function rootFetchTitle(threadId) {
  const r = await fetch('http://localhost:8666/sql', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'text/plain',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: ROOT_BASIC,
    },
    body: `SELECT title FROM ${threadId};`,
  });
  const json = await r.json();
  return json[0]?.result?.[0]?.title ?? null;
}

async function connectAndSignin(endpoint, username, password) {
  const s = new Surreal();
  await s.connect(endpoint);
  await s.use({ namespace: NS, database: DB });
  await s.signin({ access: 'account', variables: { username, password } });
  return s;
}

async function readTitle(s, threadId) {
  const [rows] = await s.query(`SELECT title FROM ${threadId};`);
  return rows?.[0]?.title ?? null;
}

function logResult(label, expected, actual) {
  const ok = expected === actual;
  const flag = ok ? '✓ FRESH' : '✗ STALE';
  console.log(
    `  ${flag}  ${label.padEnd(48)}  expected=${JSON.stringify(expected)}  got=${JSON.stringify(actual)}`
  );
  return ok;
}

// ── fixture ─────────────────────────────────────────────────────

console.log('Setting up alice + bob + a published thread …');
const alice = { username: rand('alice'), password: 'p' };
const bob = { username: rand('bob'), password: 'p' };
console.log(`  alice=${alice.username}  bob=${bob.username}`);

// Setup signups via the HTTP /signup endpoint to avoid Node 22's
// experimental-WebSocket flake. The endpoint accepts the same access
// function we use over WS.
async function signupViaHttp(user) {
  process.stdout.write(`  signup ${user.username}: `);
  const r = await fetch('http://localhost:8666/signup', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      ns: NS,
      db: DB,
      ac: 'account',
      username: user.username,
      password: user.password,
      // SurrealDB's HTTP /signup rejects nulls here even though the
      // field type is option<string>. Empty strings get coerced fine.
      share_pubkey: '',
      share_privkey: '',
    }),
  });
  const text = await r.text();
  if (!r.ok) throw new Error(`signup ${user.username} failed: ${r.status} ${text}`);
  console.log('done.');
}

await signupViaHttp(alice);
await signupViaHttp(bob);

// Alice creates a published thread — easier via direct root-curl
// since we already have those credentials and don't need a SurrealDB
// SDK connection for it. The thread's `author` field still references
// alice's user record via record id, which is what record-user
// authentication will key off downstream.
const threadId = `thread:repro_${randomBytes(4).toString('hex')}`;
{
  process.stdout.write(`  seed ${threadId}: `);
  // Look up alice's user id.
  const r1 = await fetch('http://localhost:8666/sql', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'text/plain',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: ROOT_BASIC,
    },
    body: `SELECT id FROM user WHERE username = "${alice.username}" LIMIT 1;`,
  });
  const j1 = await r1.json();
  const aliceUserId = j1[0]?.result?.[0]?.id;
  if (!aliceUserId) throw new Error(`couldn't resolve alice user id: ${JSON.stringify(j1)}`);
  const r2 = await fetch('http://localhost:8666/sql', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'text/plain',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: ROOT_BASIC,
    },
    body: `CREATE ${threadId} SET title = "v1", published = true, author = ${aliceUserId}, active = true, content = NONE;`,
  });
  const j2 = await r2.json();
  if (j2[0]?.status !== 'OK') throw new Error(`seed CREATE failed: ${JSON.stringify(j2)}`);
  console.log('done.');
}
console.log('');

// Confirm via root-curl that the seed is committed.
{
  const t = await rootFetchTitle(threadId);
  console.log(`  root-curl initial title=${JSON.stringify(t)}`);
  console.log('');
}

// ── tests ──────────────────────────────────────────────────────

const results = [];

let titleVersion = 1; // we start at "v1"

async function bumpAndExpect() {
  titleVersion += 1;
  const newTitle = `v${titleVersion}`;
  // Use root-curl to update — keeps the writer cache out of the
  // experiment, and lets us assert upstream is committed before
  // bob re-reads.
  await fetch('http://localhost:8666/sql', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'text/plain',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: ROOT_BASIC,
    },
    body: `UPDATE ${threadId} SET title = "${newTitle}";`,
  });
  const upstream = await rootFetchTitle(threadId);
  if (upstream !== newTitle) {
    throw new Error(
      `Upstream didn't accept the UPDATE? got ${JSON.stringify(upstream)}`
    );
  }
  return newTitle;
}

// ── test 1: HTTP / record-user / prior fetch primed ────────────
{
  const bobS = await connectAndSignin(HTTP, bob.username, bob.password);
  await readTitle(bobS, threadId); // prime
  const expected = await bumpAndExpect();
  const after = await readTitle(bobS, threadId);
  results.push(logResult('HTTP record-user, prior fetch primed', expected, after));
  await bobS.close();
}

// ── test 2: HTTP / record-user / NO prior fetch ────────────────
{
  const bobS = await connectAndSignin(HTTP, bob.username, bob.password);
  const expected = await bumpAndExpect();
  const after = await readTitle(bobS, threadId);
  results.push(logResult('HTTP record-user, no prior fetch', expected, after));
  await bobS.close();
}

// ── test 3: HTTP / record-user / prior fetch + re-signin ───────
{
  const bobS = await connectAndSignin(HTTP, bob.username, bob.password);
  await readTitle(bobS, threadId); // prime
  const expected = await bumpAndExpect();
  await bobS.signin({
    access: 'account',
    variables: { username: bob.username, password: bob.password },
  });
  const after = await readTitle(bobS, threadId);
  results.push(
    logResult('HTTP record-user, prior fetch + re-signin', expected, after)
  );
  await bobS.close();
}

// ── test 4: HTTP / record-user / fresh CONNECTION per fetch ────
{
  const bobS1 = await connectAndSignin(HTTP, bob.username, bob.password);
  await readTitle(bobS1, threadId); // prime bobS1
  await bobS1.close();
  const expected = await bumpAndExpect();
  const bobS2 = await connectAndSignin(HTTP, bob.username, bob.password);
  const after = await readTitle(bobS2, threadId);
  results.push(
    logResult('HTTP record-user, fresh connection per fetch', expected, after)
  );
  await bobS2.close();
}

// ── test 5: HTTP / SDK root auth / prior fetch primed ──────────
// Does root auth via the SDK also see staleness?
{
  const rootS = new Surreal();
  await rootS.connect(HTTP);
  await rootS.use({ namespace: NS, database: DB });
  await rootS.signin({ username: 'root', password: 'root' });
  await readTitle(rootS, threadId); // prime
  const expected = await bumpAndExpect();
  const after = await readTitle(rootS, threadId);
  results.push(logResult('HTTP SDK root-auth, prior fetch primed', expected, after));
  await rootS.close();
}

// ── test 6: raw HTTP /sql with record-user bearer token ────────
// Skip the SDK entirely. Sign in once to capture the token, then
// POST /sql with `Authorization: Bearer <jwt>` each fetch.
{
  const tokRes = await fetch('http://localhost:8666/signin', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      ns: NS,
      db: DB,
      ac: 'account',
      username: bob.username,
      password: bob.password,
    }),
  });
  const tok = (await tokRes.json()).token;
  const doFetch = async () => {
    const r = await fetch('http://localhost:8666/sql', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'text/plain',
        'surreal-ns': NS,
        'surreal-db': DB,
        Authorization: `Bearer ${tok}`,
      },
      body: `SELECT title FROM ${threadId};`,
    });
    const j = await r.json();
    return j[0]?.result?.[0]?.title ?? null;
  };
  await doFetch(); // prime
  const expected = await bumpAndExpect();
  const after = await doFetch();
  results.push(
    logResult('Raw HTTP /sql with bob bearer, prior prime', expected, after)
  );
}

// ── test 7: raw HTTP /sql with FRESH record-user signin per req ─
{
  const doFreshSigninAndFetch = async () => {
    const tokRes = await fetch('http://localhost:8666/signin', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ns: NS,
        db: DB,
        ac: 'account',
        username: bob.username,
        password: bob.password,
      }),
    });
    const tok = (await tokRes.json()).token;
    const r = await fetch('http://localhost:8666/sql', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'text/plain',
        'surreal-ns': NS,
        'surreal-db': DB,
        Authorization: `Bearer ${tok}`,
      },
      body: `SELECT title FROM ${threadId};`,
    });
    const j = await r.json();
    return j[0]?.result?.[0]?.title ?? null;
  };
  await doFreshSigninAndFetch(); // prime (with one token)
  const expected = await bumpAndExpect();
  const after = await doFreshSigninAndFetch(); // fresh signin → fresh token
  results.push(
    logResult('Raw HTTP /sql, fresh signin per fetch', expected, after)
  );
}

// ── ws variants ────────────────────────────────────────────────
// Mirror the key HTTP cases over the WS engine, now that the `ws`
// shim is in place. If the staleness is WS-only, these should fail
// while their HTTP counterparts above succeed.

// ── test W1: WS / record-user / prior fetch primed ─────────────
{
  const bobS = await connectAndSignin(WS, bob.username, bob.password);
  await readTitle(bobS, threadId); // prime
  const expected = await bumpAndExpect();
  const after = await readTitle(bobS, threadId);
  results.push(logResult('WS record-user, prior fetch primed', expected, after));
  await bobS.close();
}

// ── test W2: WS / record-user / NO prior fetch ─────────────────
{
  const bobS = await connectAndSignin(WS, bob.username, bob.password);
  const expected = await bumpAndExpect();
  const after = await readTitle(bobS, threadId);
  results.push(logResult('WS record-user, no prior fetch', expected, after));
  await bobS.close();
}

// ── test W3: WS / record-user / prior fetch + LIVE primed ──────
// LIVE subscription on the parent table BEFORE the prime read,
// to see if LIVE primes any session-level snapshot for the row.
{
  const bobS = await connectAndSignin(WS, bob.username, bob.password);
  await bobS.query(`LIVE SELECT * FROM thread;`);
  await readTitle(bobS, threadId); // prime
  const expected = await bumpAndExpect();
  // Let the LIVE event flow in (or not).
  await new Promise((r) => setTimeout(r, 500));
  const after = await readTitle(bobS, threadId);
  results.push(
    logResult('WS record-user, LIVE + prior fetch primed', expected, after)
  );
  await bobS.close();
}

// ── test W3b: WS / record-user / BOUND $idsToFetch ─────────────
// Match the exact syncEngine.syncRecords query shape:
// `SELECT * FROM $idsToFetch` with idsToFetch bound as an array of
// RecordId objects. The in-app failure path uses this form.
{
  const { RecordId: SDKRecordId } = await import('surrealdb');
  const bobS = await connectAndSignin(WS, bob.username, bob.password);
  const [tableName, recordKey] = threadId.split(':');
  const rid = new SDKRecordId(tableName, recordKey);

  // Prime once via the same bound shape.
  const [primeRows] = await bobS.query('SELECT * FROM $idsToFetch', {
    idsToFetch: [rid],
  });
  void primeRows;

  const expected = await bumpAndExpect();

  const [afterRows] = await bobS.query('SELECT * FROM $idsToFetch', {
    idsToFetch: [rid],
  });
  const after = afterRows?.[0]?.title ?? null;
  results.push(
    logResult('WS record-user, BOUND $idsToFetch (in-app shape)', expected, after)
  );
  await bobS.close();
}

// ── test W3c: WS / record-user / BOUND $idsToFetch + LIVE on list_ref ─
// Real-app shape: LIVE is on `_00_list_ref_user_<id>`, not thread.
// The list_ref tables exist from earlier app runs; try one.
{
  const { RecordId: SDKRecordId } = await import('surrealdb');
  const bobS = await connectAndSignin(WS, bob.username, bob.password);
  const [tableName, recordKey] = threadId.split(':');
  const rid = new SDKRecordId(tableName, recordKey);

  // Find a list_ref table to LIVE on. Pick the first that exists.
  // If none, skip the LIVE part — the test still reproduces the shape.
  let liveTable = null;
  try {
    const [tables] = await bobS.query(
      "INFO FOR DB"
    );
    const tbls = tables?.tables ?? {};
    liveTable = Object.keys(tbls).find((k) => k.startsWith('_00_list_ref_user_'));
  } catch {
    // ignore
  }
  if (liveTable) {
    try {
      await bobS.query(`LIVE SELECT * FROM ${liveTable};`);
    } catch {
      // permission may deny LIVE for bob; that's OK for this test
    }
  }

  // Prime via bound shape.
  await bobS.query('SELECT * FROM $idsToFetch', { idsToFetch: [rid] });
  const expected = await bumpAndExpect();
  await new Promise((r) => setTimeout(r, 100));
  const [afterRows] = await bobS.query('SELECT * FROM $idsToFetch', {
    idsToFetch: [rid],
  });
  const after = afterRows?.[0]?.title ?? null;
  results.push(
    logResult(
      `WS record-user, bound + LIVE on ${liveTable ?? '(none)'}`,
      expected,
      after
    )
  );
  await bobS.close();
}

// ── test W4: WS / SDK root-auth / prior fetch primed ───────────
{
  const rootS = new Surreal();
  await rootS.connect(WS);
  await rootS.use({ namespace: NS, database: DB });
  await rootS.signin({ username: 'root', password: 'root' });
  await readTitle(rootS, threadId); // prime
  const expected = await bumpAndExpect();
  const after = await readTitle(rootS, threadId);
  results.push(logResult('WS SDK root-auth, prior fetch primed', expected, after));
  await rootS.close();
}

// ── test 8: raw HTTP /sql with bob bearer, NO prior fetch ──────
{
  const tokRes = await fetch('http://localhost:8666/signin', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      ns: NS,
      db: DB,
      ac: 'account',
      username: bob.username,
      password: bob.password,
    }),
  });
  const tok = (await tokRes.json()).token;
  // First mutation is the bump, FIRST read is the assertion.
  const expected = await bumpAndExpect();
  const r = await fetch('http://localhost:8666/sql', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'text/plain',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: `Bearer ${tok}`,
    },
    body: `SELECT title FROM ${threadId};`,
  });
  const j = await r.json();
  const after = j[0]?.result?.[0]?.title ?? null;
  results.push(
    logResult('Raw HTTP /sql with bob bearer, NO prior fetch', expected, after)
  );
}

// ── summary ────────────────────────────────────────────────────

console.log('');
const passed = results.filter(Boolean).length;
console.log(`${passed}/${results.length} returned fresh data after UPDATE.`);
console.log('Failing cases isolate the bug shape.');

process.exit(0);
