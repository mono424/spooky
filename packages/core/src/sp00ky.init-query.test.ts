import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Sp00kyClient } from './sp00ky';

// Guards for the local-first paint contract of `Sp00kyClient.initQuery`:
// the hash must resolve (and the hook paint from local cache) without any
// network on the awaited path, while the opt-in instant-hydrate + the
// `register` down-event run in a background chain that (a) keeps hydrate
// strictly before the enqueue, (b) hydrates only when `instantHydrate:
// true` (default off — the register lifecycle is the single freshness
// path), (c) never rejects, and (d) is shared by concurrent mounts of the
// same query.
//
// Structural half: like sp00ky.auth-order.test.ts, a regex over the source
// catches the ordering regressions a runtime mock can't cheaply cover
// (Sp00kyClient's constructor drags in SurrealDB + the WASM SSP).
// Behavioral half: `finishQueryInit`/`initQuery` only touch a handful of
// injected fields, so a bare `Object.create(Sp00kyClient.prototype)` with
// stubs exercises the real method bodies without the constructor.

describe('Sp00kyClient.initQuery structural invariants', () => {
  const source = readFileSync(resolve(__dirname, 'sp00ky.ts'), 'utf-8');

  const methodBody = (name: string): string => {
    const match = source.match(new RegExp(`private async ${name}[\\s\\S]*?\\n  \\}`));
    expect(match, `expected a "private async ${name}" method in sp00ky.ts`).not.toBeNull();
    // Strip line comments so prose mentioning awaits/calls can't trip the checks.
    return match![0]
      .split('\n')
      .map((line) => line.replace(/\/\/.*$/, ''))
      .join('\n');
  };

  it('initQuery returns the hash with no network on the awaited path', () => {
    const body = methodBody('initQuery');
    expect(body).toContain('return hash');
    expect(body, 'initQuery must not await the remote (paint path is network-free)').not.toMatch(
      /await this\.remote\./
    );
    expect(body, 'the register enqueue belongs to the background chain').not.toMatch(
      /await this\.sync\.enqueueDownEvent/
    );
  });

  it('finishQueryInit enqueues register before the hydrate fetch', () => {
    const body = methodBody('finishQueryInit');
    const fetchIdx = body.indexOf('this.remote.query');
    const enqueueIdx = body.indexOf('this.sync.enqueueDownEvent');
    expect(fetchIdx).toBeGreaterThanOrEqual(0);
    expect(enqueueIdx).toBeGreaterThanOrEqual(0);
    expect(
      enqueueIdx,
      'the authoritative register must never wait on the one-shot hydrate read (a windowed list paid a full-table scan per window before its own registration could start); applyHydration drops a hydrate that lands after the registration'
    ).toBeLessThan(fetchIdx);
  });

  it('finishQueryInit captures the bucket epoch before the remote fetch', () => {
    const body = methodBody('finishQueryInit');
    const epochIdx = body.indexOf('this.local.epoch');
    const fetchIdx = body.indexOf('this.remote.query');
    expect(epochIdx).toBeGreaterThanOrEqual(0);
    expect(epochIdx, 'epoch must be read before the fetch to fence bucket switches').toBeLessThan(
      fetchIdx
    );
  });
});

describe('Sp00kyClient.finishQueryInit behavior', () => {
  const hash = 'sha-hash';
  const q: any = { hash: 42, selectQuery: { query: 'SELECT * FROM user', vars: {} } };

  let client: any;
  let calls: string[];
  let enqueued: any[];
  let epoch: number;
  let cold: boolean;
  let remoteImpl: () => Promise<any>;

  beforeEach(() => {
    calls = [];
    enqueued = [];
    epoch = 1;
    cold = true;
    remoteImpl = async () => {
      calls.push('fetch');
      return [[{ id: 'user:a' }]];
    };

    const noop = () => {};
    const logger: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
    logger.child = () => logger;

    client = Object.create(Sp00kyClient.prototype);
    Object.assign(client, {
      config: { instantHydrate: true, schema: { tables: [{ name: 'user', columns: {} }] } },
      logger,
      preloadedHashes: new Set<number>(),
      pendingQueryInits: new Map<string, Promise<void>>(),
      local: {
        get epoch() {
          return epoch;
        },
      },
      remote: { query: (...args: any[]) => remoteImpl() },
      sync: {
        enqueueDownEvent: (event: any) => {
          calls.push('enqueue');
          enqueued.push(event);
        },
      },
      dataModule: {
        isCold: () => cold,
        applyHydration: async () => {
          calls.push('hydrate');
        },
        query: async () => hash,
      },
    });
  });

  it('cold path with hydrate enabled: enqueue → fetch → hydrate, in order', async () => {
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue', 'fetch', 'hydrate']);
    expect(enqueued).toEqual([{ type: 'register', payload: { hash } }]);
  });

  it('a preloaded query still hydrates when enabled — cache-first never depends on WHY rows are cached', async () => {
    client.preloadedHashes.add(q.hash);
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue', 'fetch', 'hydrate']);
  });

  it('warm query (not cold) goes straight to enqueue', async () => {
    cold = false;
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue']);
  });

  it('a rejected hydrate fetch still enqueues register and never rejects', async () => {
    remoteImpl = async () => {
      calls.push('fetch');
      throw new Error('offline');
    };
    await expect(client.finishQueryInit(hash, q, {})).resolves.toBeUndefined();
    expect(calls).toEqual(['enqueue', 'fetch']);
  });

  it('a bucket switch during the fetch skips applyHydration', async () => {
    remoteImpl = async () => {
      calls.push('fetch');
      epoch = 2; // switch lands while the fetch is in flight
      return [[{ id: 'user:a' }]];
    };
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue', 'fetch']);
  });

  it('default (instantHydrate unset) does not hydrate — register lifecycle is the only freshness path', async () => {
    delete client.config.instantHydrate;
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue']);
  });

  it('instantHydrate: false does not hydrate', async () => {
    client.config.instantHydrate = false;
    await client.finishQueryInit(hash, q, {});
    expect(calls).toEqual(['enqueue']);
  });

  it('concurrent initQuery calls for the same hash share one background chain', async () => {
    const [h1, h2] = await Promise.all([
      client.initQuery('user', q, '10m'),
      client.initQuery('user', q, '10m'),
    ]);
    expect(h1).toBe(hash);
    expect(h2).toBe(hash);
    // Let the shared chain drain.
    await Promise.all([...client.pendingQueryInits.values()]);
    expect(calls).toEqual(['enqueue', 'fetch', 'hydrate']);
    expect(client.pendingQueryInits.size).toBe(0);
  });
});
