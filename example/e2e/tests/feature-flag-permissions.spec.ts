import { test, expect } from '@playwright/test';

/**
 * Permission negative tests for the built-in feature flag tables.
 *
 * The two system tables added by `apps/cli/src/meta_tables_remote.surql`
 * are the foundation of the feature flag system, so their permission
 * guarantees are load-bearing:
 *
 *   _00_feature_flag   - PERMISSIONS NONE: record-token clients cannot
 *                        read targeting rules or write definitions.
 *   _00_user_feature   - select scoped to WHERE user = $auth.id;
 *                        create/update/delete denied entirely so a
 *                        client cannot self-enable a flag.
 *
 * These tests run against the upstream SurrealDB directly (the same
 * pattern as `waitForThreadInUpstream` in `test-fixtures.ts`).
 */

const SURREAL_URL = 'http://localhost:8666';
const NS = 'main';
const DB = 'example';
const ROOT_AUTH = 'Basic ' + Buffer.from('root:root').toString('base64');

async function rootSql(query: string): Promise<unknown> {
  const resp = await fetch(`${SURREAL_URL}/sql`, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: ROOT_AUTH,
    },
    body: query,
  });
  expect(resp.ok, `root SQL failed: ${resp.status}`).toBeTruthy();
  return resp.json();
}

async function signupUser(
  username: string,
  password: string
): Promise<string> {
  const resp = await fetch(`${SURREAL_URL}/signup`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({
      ns: NS,
      db: DB,
      ac: 'account',
      username,
      password,
    }),
  });
  expect(resp.ok, `signup failed: ${resp.status}`).toBeTruthy();
  const body = (await resp.json()) as { token?: string };
  expect(body.token, 'signup did not return a token').toBeTruthy();
  return body.token as string;
}

async function recordSql(token: string, query: string): Promise<{
  status: number;
  body: Array<{ status: string; result?: unknown }>;
}> {
  const resp = await fetch(`${SURREAL_URL}/sql`, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'surreal-ns': NS,
      'surreal-db': DB,
      Authorization: `Bearer ${token}`,
    },
    body: query,
  });
  const status = resp.status;
  const body = (await resp.json()) as Array<{
    status: string;
    result?: unknown;
  }>;
  return { status, body };
}

function uniq(prefix: string) {
  return `${prefix}_${Date.now().toString(36)}${Math.random()
    .toString(36)
    .slice(2, 6)}`;
}

test.describe.serial('Feature flag permissions', () => {
  let aliceToken = '';
  let bobToken = '';
  let aliceId = '';
  let bobId = '';
  const flagKey = uniq('e2e_flag');

  test.beforeAll(async () => {
    const aliceName = uniq('alice');
    const bobName = uniq('bob');
    aliceToken = await signupUser(aliceName, 'pw_alice_1234');
    bobToken = await signupUser(bobName, 'pw_bob_1234');

    const aliceLookup = (await rootSql(
      `SELECT VALUE id FROM user WHERE username = '${aliceName}' LIMIT 1;`
    )) as Array<{ result?: string[] }>;
    aliceId = aliceLookup[0]?.result?.[0] ?? '';
    const bobLookup = (await rootSql(
      `SELECT VALUE id FROM user WHERE username = '${bobName}' LIMIT 1;`
    )) as Array<{ result?: string[] }>;
    bobId = bobLookup[0]?.result?.[0] ?? '';
    expect(aliceId, 'failed to resolve alice id').toBeTruthy();
    expect(bobId, 'failed to resolve bob id').toBeTruthy();
  });

  test.afterAll(async () => {
    if (flagKey) {
      await rootSql(`DELETE _00_feature_flag WHERE key = '${flagKey}';`);
      await rootSql(`DELETE _00_user_feature WHERE key = '${flagKey}';`);
    }
  });

  test('record-token client cannot CREATE _00_user_feature', async () => {
    const { body } = await recordSql(
      aliceToken,
      `CREATE _00_user_feature SET user = ${aliceId}, key = '${flagKey}', variant = 'on';`
    );
    const statuses = body.map((r) => r.status);
    expect(
      statuses.every((s) => s === 'ERR') ||
        body.every((r) => Array.isArray(r.result) && r.result.length === 0),
      `expected denial, got: ${JSON.stringify(body)}`
    ).toBeTruthy();

    const after = (await rootSql(
      `SELECT count() AS n FROM _00_user_feature WHERE key = '${flagKey}' GROUP ALL;`
    )) as Array<{ result?: Array<{ n: number }> }>;
    const n = after[0]?.result?.[0]?.n ?? 0;
    expect(n, 'self-enable mutation must not have persisted any row').toBe(0);
  });

  test('record-token client cannot UPDATE _00_feature_flag', async () => {
    await rootSql(
      `CREATE _00_feature_flag SET key = '${flagKey}', variants = ['off','on'], default_variant = 'off', rules = [], enabled = true;`
    );

    const { body } = await recordSql(
      aliceToken,
      `UPDATE _00_feature_flag SET enabled = false WHERE key = '${flagKey}';`
    );
    const statuses = body.map((r) => r.status);
    expect(
      statuses.every((s) => s === 'ERR') ||
        body.every((r) => Array.isArray(r.result) && r.result.length === 0),
      `expected denial, got: ${JSON.stringify(body)}`
    ).toBeTruthy();

    const after = (await rootSql(
      `SELECT VALUE enabled FROM _00_feature_flag WHERE key = '${flagKey}';`
    )) as Array<{ result?: boolean[] }>;
    expect(after[0]?.result?.[0], 'flag.enabled must not have changed').toBe(true);
  });

  test('record-token client cannot SELECT _00_feature_flag', async () => {
    const { body } = await recordSql(
      aliceToken,
      `SELECT * FROM _00_feature_flag WHERE key = '${flagKey}';`
    );
    const rows = (body[0]?.result ?? []) as unknown[];
    expect(
      rows.length,
      'targeting rules must not be visible to record-token clients'
    ).toBe(0);
  });

  test('record-token client only sees its own _00_user_feature rows', async () => {
    await rootSql(
      `UPSERT _00_user_feature WHERE user = ${aliceId} AND key = '${flagKey}' SET user = ${aliceId}, key = '${flagKey}', variant = 'on';`
    );
    await rootSql(
      `UPSERT _00_user_feature WHERE user = ${bobId} AND key = '${flagKey}' SET user = ${bobId}, key = '${flagKey}', variant = 'off';`
    );

    const { body: aliceBody } = await recordSql(
      aliceToken,
      `SELECT user, variant FROM _00_user_feature WHERE key = '${flagKey}';`
    );
    const aliceRows = (aliceBody[0]?.result ?? []) as Array<{
      user?: string;
      variant?: string;
    }>;
    expect(aliceRows.length, 'alice sees exactly one row').toBe(1);
    expect(aliceRows[0]?.variant).toBe('on');

    const { body: bobBody } = await recordSql(
      bobToken,
      `SELECT user, variant FROM _00_user_feature WHERE key = '${flagKey}';`
    );
    const bobRows = (bobBody[0]?.result ?? []) as Array<{
      user?: string;
      variant?: string;
    }>;
    expect(bobRows.length, 'bob sees exactly one row').toBe(1);
    expect(bobRows[0]?.variant).toBe('off');
  });
});
