import { describe, it, expect, beforeEach } from 'vitest';
import { FeatureFlagModule } from './index';

// Minimal mocks for the three deps the module touches. The DataModule mock
// captures the single subscribe callback so a test can push live results, and
// counts query() calls to assert the query is SHARED (one registration for all
// flags), not per-key.
function makeDeps() {
  let subCb: ((records: unknown[]) => void) | null = null;
  const calls: Array<{ sql: string; params: unknown }> = [];
  let authCb: ((userId: string | null) => void) | null = null;

  const dataModule = {
    query: async (_table: string, sql: string, params: unknown) => {
      calls.push({ sql, params });
      return `hash:${calls.length}`;
    },
    subscribe: (_hash: string, cb: (records: unknown[]) => void) => {
      subCb = cb;
      return () => {
        subCb = null;
      };
    },
  };
  const sync = { enqueueDownEvent: () => {} };
  const auth = {
    subscribe: (cb: (userId: string | null) => void) => {
      authCb = cb;
      return () => {
        authCb = null;
      };
    },
  };
  const logger = { child: () => ({ warn: () => {} }) };

  const deps = { dataModule, sync, auth, logger } as any;
  return {
    deps,
    calls,
    push: (records: unknown[]) => subCb?.(records),
    setUser: (id: string | null) => authCb?.(id),
    hasSub: () => subCb !== null,
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('FeatureFlagModule', () => {
  let env: ReturnType<typeof makeDeps>;
  let mod: FeatureFlagModule<any>;

  beforeEach(() => {
    env = makeDeps();
    mod = new FeatureFlagModule(env.deps);
  });

  it('registers ONE shared, unfiltered query for many flags', async () => {
    mod.feature('alpha');
    mod.feature('beta');
    mod.feature('gamma');
    await tick();

    expect(env.calls.length).toBe(1);
    expect(env.calls[0].sql).not.toContain('WHERE');
    expect(env.calls[0].sql).toContain('FROM _00_user_feature');
    expect(env.calls[0].params).toEqual({});
  });

  it('fans the shared result out to each handle by key', async () => {
    const alpha = mod.feature('alpha', { fallback: 'off' });
    const beta = mod.feature('beta', { fallback: 'off' });
    const missing = mod.feature('missing', { fallback: 'off' });
    await tick();

    env.push([
      { key: 'alpha', variant: 'on' },
      { key: 'beta', variant: 'off' },
    ]);

    expect(alpha.enabled()).toBe(true);
    expect(alpha.variant()).toBe('on');
    expect(beta.enabled()).toBe(false); // assigned 'off'
    expect(missing.enabled()).toBe(false); // no row → fallback 'off'
  });

  it('observes NEW assignments live without re-registering', async () => {
    const flag = mod.feature('live', { fallback: 'off' });
    await tick();

    env.push([]); // user starts with no assignment
    expect(flag.enabled()).toBe(false);

    env.push([{ key: 'live', variant: 'on' }]); // assigned while observing
    expect(flag.enabled()).toBe(true);

    expect(env.calls.length).toBe(1); // still the same single query
  });

  it('seeds a late-created handle from the already-loaded snapshot', async () => {
    mod.feature('alpha', { fallback: 'off' });
    await tick();
    env.push([{ key: 'alpha', variant: 'on' }]);

    const late = mod.feature('alpha', { fallback: 'off' });
    expect(late.enabled()).toBe(true); // no fallback flash
  });

  it('clears flags and re-observes on user change', async () => {
    mod.init();
    const flag = mod.feature('alpha', { fallback: 'off' });
    await tick();
    env.push([{ key: 'alpha', variant: 'on' }]);
    expect(flag.enabled()).toBe(true);

    env.setUser('user:other'); // sign-in as a different user
    await tick();
    expect(flag.enabled()).toBe(false); // cleared until the new query resolves
    expect(env.hasSub()).toBe(true); // re-registered for the new user
  });
});
