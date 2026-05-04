import { OpenAPIHono, createRoute, z as zOpenApi } from '@hono/zod-openapi';
import { cors } from 'hono/cors';
import { logger } from 'hono/logger';
import { bearerAuth } from 'hono/bearer-auth';
import { z } from 'zod';
import { generateText, Output } from 'ai';
import { anthropic } from '@ai-sdk/anthropic';
import { DateTime, RecordId, Surreal } from 'surrealdb';
import * as jose from 'jose';

const model = anthropic('claude-haiku-4-5');

const app = new OpenAPIHono();

app.use(logger());
// Browser-driven endpoints (e.g. /share/accept) need CORS for the dev port.
app.use('/*', cors({ origin: '*', allowHeaders: ['Authorization', 'Content-Type'], allowMethods: ['POST', 'OPTIONS'] }));

// Static-secret bearer auth gates the AI / spooky routes only. The
// /share/accept route is excluded — it authenticates the caller via their
// own SurrealDB session token instead.
app.use(
  '/*',
  async (c, next) => {
    if (c.req.path.startsWith('/share/')) return next();
    return bearerAuth({
      token: process.env.API_AUTH_TOKEN || 'THIS_IS_TOP_SECRET',
    })(c, next);
  }
);

const db = new Surreal({
  codecOptions: {
    valueDecodeVisitor(value) {
      if (value instanceof RecordId) {
        return value.table.name + ':' + value.id.toString();
      }

      if (value instanceof DateTime) {
        return value.toDate();
      }

      return value;
    },
  },
});

const ErrorSchema = zOpenApi.object({
  error: zOpenApi.string(),
});

const spookifyRoute = createRoute({
  method: 'post',
  path: '/spookify',
  request: {
    body: {
      content: {
        'application/json': {
          schema: zOpenApi.object({
            id: zOpenApi.string().openapi({
              example: 'thread:kv9b3b...',
            }),
          }),
        },
      },
    },
  },
  responses: {
    200: {
      description: 'ok',
    },
    404: {
      content: {
        'application/json': {
          schema: ErrorSchema,
        },
      },
      description: 'Thread not found',
    },
    500: {
      content: {
        'application/json': {
          schema: ErrorSchema,
        },
      },
      description: 'Internal server error',
    },
  },
});

const parseId = (id: string) => {
  if (id.startsWith('thread:')) {
    return new RecordId(id.substring(0, 6), id.substring(7));
  }
  const [table, ...rest] = id.split(':');
  if (table !== 'thread') {
    throw new Error('Invalid table');
  }
  if (!rest.length) {
    throw new Error('Invalid id');
  }
  return new RecordId(table, rest.join(':'));
};

app.openapi(spookifyRoute, async (c) => {
  const { id } = c.req.valid('json');
  const recordId = parseId(id);

  try {
    // Connect to SurrealDB
    const surrealUrl = process.env.SPKY_DB_URL || 'http://127.0.0.1:8000/rpc';
    await db.connect(surrealUrl);

    await db.use({
      namespace: process.env.SPKY_DB_NS || 'main',
      database: process.env.SPKY_DB_NAME || 'example',
    });

    await db.signin({
      username: process.env.SPKY_DB_USER || 'root',
      password: process.env.SPKY_DB_PASS || 'root',
    });

    // 1. Query the record
    // db.query returns Promise<T>. For one statement, T should be [ResultType]
    // ResultType is an array of records.
    type ThreadRecord = { id: string; title: string; content: string };
    const [result] = await db.query<[ThreadRecord[]]>('SELECT id, title, content FROM $id', {
      id: recordId,
    });

    // result is ThreadRecord[]
    const record = result && result.length > 0 ? result[0] : null;

    if (!record) {
      return c.json({ error: 'Thread not found' }, 404);
    }

    // 2. Generate sp00ky content with AI
    // Warning: Requires ANTHROPIC_API_KEY environment variable
    // Use z from 'zod' here
    // Define the output structure
    type Sp00kySuggestion = {
      title_suggestion: string;
      content_suggestion: string;
    };

    const resultSchema = z.object({
      title_suggestion: z.string().describe('A sp00ky version of the original title'),
      content_suggestion: z.string().describe('A sp00ky, eerie version of the original content'),
    }) as z.Schema<Sp00kySuggestion>;

    const resultAI = await generateText({
      model,
      output: Output.object({
        schema: resultSchema as any,
      }),
      prompt: `Sp00kify the following thread content. Make it sound haunted, eerie, and fit for a ghost story.
      
      Original Title: ${record.title}
      Original Content: ${record.content}
      `,
    });

    if (!resultAI.output) {
      throw new Error('Failed to generate output');
    }

    const { title_suggestion, content_suggestion } = resultAI.output as z.infer<
      typeof resultSchema
    >;

    // 3. Update the record
    await db.update(recordId).merge({
      title_suggestion,
      content_suggestion,
    });

    c.status(200);
    return c.text('ok');
  } catch (e: any) {
    console.error(e);
    return c.json({ error: e.message || 'Internal Server Error' }, 500);
  }
});

// ============================================================================
// Share-link acceptance
//
// The issuer signs a JWT locally with their Ed25519 private key and gives
// out `/invite/<jwt>`. The recipient hits this endpoint with their Surreal
// session token in `Authorization`; we identify them via that token, fetch
// the issuer's `share_pubkey` (root creds), verify the JWT, and RELATE
// `recipient -> collaborates_on -> thread` as root. The relation table's
// CREATE rule is `false`, so this is the only path that adds a collaborator.
// ============================================================================

const shareAcceptResponse = zOpenApi.object({ thread: zOpenApi.string() });
const shareAcceptError = zOpenApi.object({ error: zOpenApi.string() });

const shareAcceptRoute = createRoute({
  method: 'post',
  path: '/share/accept',
  request: {
    body: {
      content: {
        'application/json': {
          schema: zOpenApi.object({ jwt: zOpenApi.string() }),
        },
      },
    },
  },
  responses: {
    200: { content: { 'application/json': { schema: shareAcceptResponse } }, description: 'Joined thread.' },
    400: { content: { 'application/json': { schema: shareAcceptError } }, description: 'Malformed JWT.' },
    401: { content: { 'application/json': { schema: shareAcceptError } }, description: 'Missing or invalid recipient session.' },
    403: { content: { 'application/json': { schema: shareAcceptError } }, description: 'Invite invalid, expired, or signed by an unknown issuer.' },
    500: { content: { 'application/json': { schema: shareAcceptError } }, description: 'Internal error.' },
  },
});

const surrealUrl = () => process.env.SPKY_DB_URL || 'http://127.0.0.1:8000/rpc';
const surrealNs = () => process.env.SPKY_DB_NS || 'main';
const surrealDb = () => process.env.SPKY_DB_NAME || 'example';

async function ensureRootDb() {
  // The shared `db` may already be connected from /spookify; signing in as
  // root here is idempotent enough for our needs.
  await db.connect(surrealUrl());
  await db.use({ namespace: surrealNs(), database: surrealDb() });
  await db.signin({
    username: process.env.SPKY_DB_USER || 'root',
    password: process.env.SPKY_DB_PASS || 'root',
  });
}

app.openapi(shareAcceptRoute, async (c) => {
  const auth = c.req.header('Authorization');
  const recipientToken = auth?.replace(/^Bearer\s+/i, '').trim();
  if (!recipientToken) return c.json({ error: 'missing recipient session token' }, 401);

  const { jwt: shareJwt } = c.req.valid('json');

  // 1. Identify the recipient via their own Surreal session.
  const recipient = new Surreal();
  let recipientId: RecordId;
  try {
    await recipient.connect(surrealUrl());
    await recipient.use({ namespace: surrealNs(), database: surrealDb() });
    await recipient.authenticate(recipientToken);
    const [me] = await recipient.query<[Array<{ id: RecordId }>]>(
      'SELECT VALUE id FROM ONLY $auth.id LIMIT 1'
    );
    // SurrealDB returns the resolved id directly when SELECT VALUE id … LIMIT 1
    // returns one row; normalise both shapes.
    const idRaw: any = Array.isArray(me) ? me[0] : me;
    if (idRaw == null) return c.json({ error: 'invalid recipient session' }, 401);
    recipientId = idRaw instanceof RecordId
      ? idRaw
      : (() => {
          const s = String(idRaw);
          const idx = s.indexOf(':');
          if (idx <= 0) throw new Error(`unparseable id ${s}`);
          return new RecordId(s.slice(0, idx), s.slice(idx + 1));
        })();
  } catch (e: any) {
    return c.json({ error: e?.message || 'invalid recipient session' }, 401);
  } finally {
    await recipient.close().catch(() => {});
  }

  // 2. Verify the share JWT.
  let claims: jose.JWTPayload;
  try {
    claims = jose.decodeJwt(shareJwt);
  } catch {
    return c.json({ error: 'malformed jwt' }, 400);
  }
  const iss = typeof claims.iss === 'string' ? claims.iss : null;
  const sub = typeof claims.sub === 'string' ? claims.sub : null;
  if (!iss || !sub) return c.json({ error: 'jwt missing iss/sub' }, 400);

  try {
    await ensureRootDb();
    const issIdx = iss.indexOf(':');
    if (issIdx <= 0) return c.json({ error: 'malformed iss' }, 400);
    const issuerId = new RecordId(iss.slice(0, issIdx), iss.slice(issIdx + 1));
    const [pubRows] = await db.query<[string | null]>(
      'SELECT VALUE share_pubkey FROM ONLY $id',
      { id: issuerId }
    );
    if (!pubRows || typeof pubRows !== 'string') {
      return c.json({ error: 'unknown issuer' }, 403);
    }
    const pub = await jose.importSPKI(pubRows, 'EdDSA');
    await jose.jwtVerify(shareJwt, pub, { algorithms: ['EdDSA'] });
  } catch (e: any) {
    // jose throws specific error names: JWTExpired, JWSSignatureVerificationFailed, …
    return c.json({ error: e?.message || 'jwt verification failed' }, 403);
  }

  // 3. RELATE as root. Idempotent on the unique (in, out) index.
  try {
    const subIdx = sub.indexOf(':');
    if (subIdx <= 0) return c.json({ error: 'malformed sub' }, 400);
    const threadId = new RecordId(sub.slice(0, subIdx), sub.slice(subIdx + 1));
    await db.query('RELATE $r->collaborates_on->$t', { r: recipientId, t: threadId });
  } catch (e: any) {
    const msg = String(e?.message || e);
    if (!/already exists|unique|duplicate/i.test(msg)) {
      console.error('[share/accept] relate failed:', e);
      return c.json({ error: 'failed to record collaboration' }, 500);
    }
  }

  return c.json({ thread: sub });
});

export default app;
