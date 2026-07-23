import { describe, it, expect, beforeEach } from 'vitest';
import { AppReleaseModule } from './index';

// Mirrors the FeatureFlagModule test rig: the DataModule mock captures the
// single subscribe callback so tests can push live results, and counts
// query() calls to assert the query is SHARED across apps.
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

describe('AppReleaseModule', () => {
  let env: ReturnType<typeof makeDeps>;
  let mod: AppReleaseModule<any>;

  beforeEach(() => {
    env = makeDeps();
    mod = new AppReleaseModule(env.deps);
  });

  it('registers ONE shared, unfiltered query for many apps', async () => {
    mod.release('web');
    mod.release('admin');
    await tick();

    expect(env.calls.length).toBe(1);
    expect(env.calls[0].sql).not.toContain('WHERE');
    expect(env.calls[0].sql).toContain('FROM _00_app_release');
  });

  it('fans the shared result out to each handle by app', async () => {
    const web = mod.release('web');
    const missing = mod.release('missing');
    await tick();

    env.push([{ app: 'web', version: '1.2.0', cache_bust: true, mandatory: null }]);

    expect(web.version()).toBe('1.2.0');
    expect(web.snapshot().cacheBust).toBe(true);
    expect(web.snapshot().mandatory).toBe(false);
    expect(missing.version()).toBeUndefined();
    expect(missing.updateAvailable('1.0.0')).toBe(false);
  });

  it('updateAvailable compares semver against the running build', async () => {
    const web = mod.release('web');
    await tick();

    env.push([{ app: 'web', version: '1.2.0' }]);
    expect(web.updateAvailable('1.1.9')).toBe(true);
    expect(web.updateAvailable('1.2.0')).toBe(false);
    expect(web.updateAvailable('1.3.0')).toBe(false);
    expect(web.updateAvailable('garbage')).toBe(false);
  });

  it('observes row updates live without re-registering', async () => {
    const web = mod.release('web');
    await tick();

    env.push([{ app: 'web', version: '1.0.0' }]);
    expect(web.updateAvailable('1.0.0')).toBe(false);

    env.push([{ app: 'web', version: '1.0.1', mandatory: true }]);
    expect(web.updateAvailable('1.0.0')).toBe(true);
    expect(web.snapshot().mandatory).toBe(true);

    expect(env.calls.length).toBe(1);
  });

  it('seeds a late-created handle from the already-loaded snapshot', async () => {
    mod.release('web');
    await tick();
    env.push([{ app: 'web', version: '2.0.0' }]);

    const late = mod.release('web');
    expect(late.version()).toBe('2.0.0');
  });

  it('re-registers on user change', async () => {
    mod.init();
    const web = mod.release('web');
    await tick();
    env.push([{ app: 'web', version: '1.0.0' }]);
    expect(web.version()).toBe('1.0.0');

    env.setUser('user:other');
    await tick();
    expect(env.hasSub()).toBe(true); // re-registered under the new session
  });
});
