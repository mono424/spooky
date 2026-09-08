import { describe, expect, it, vi } from 'vitest';
import { createEffect, createRoot, createSignal, flush } from 'solid-js';
import { createQuery } from '../create-query';
import { SyncedDb } from '../../index';

const tick = () => new Promise<void>((r) => setTimeout(r, 0));
const settle = async () => {
  await tick();
  flush();
  await tick();
};

type Emission = Record<string, any>[];

function mockEngine() {
  const subs = new Map<string, (e: Emission) => void>();
  const statusSubs = new Map<string, (s: string) => void>();
  const authoritySubs = new Map<string, (known: boolean) => void>();
  const unsubscribed: string[] = [];
  const deregistered: string[] = [];
  const sp00ky = {
    subscribeQueryAuthority: vi.fn(
      (hash: string, cb: (known: boolean) => void, o?: { immediate?: boolean }) => {
        authoritySubs.set(hash, cb);
        if (o?.immediate) cb(false);
        return () => authoritySubs.delete(hash);
      }
    ),
    subscribe: vi.fn(async (hash: string, cb: (e: Emission) => void, _o?: unknown) => {
      subs.set(hash, cb);
      return () => {
        subs.delete(hash);
        unsubscribed.push(hash);
      };
    }),
    subscribeQueryStatus: vi.fn(
      (hash: string, cb: (s: string) => void, o?: { immediate?: boolean }) => {
        statusSubs.set(hash, cb);
        if (o?.immediate) cb('idle');
        return () => statusSubs.delete(hash);
      }
    ),
    deregisterQuery: vi.fn((hash: string) => deregistered.push(hash)),
    reportFrontendTiming: vi.fn(),
  };
  const db = Object.create(SyncedDb.prototype) as SyncedDb<any>;
  (db as any).getSp00ky = () => sp00ky;
  return {
    db,
    sp00ky,
    emit: (hash: string, e: Emission) => subs.get(hash)?.(e),
    setStatus: (hash: string, s: string) => statusSubs.get(hash)?.(s),
    setAuthority: (hash: string, known: boolean) => authoritySubs.get(hash)?.(known),
    unsubscribed,
    deregistered,
    hasSub: (hash: string) => subs.has(hash),
  };
}

function mockQuery(hash: string, isOne = false) {
  return {
    hash,
    isOne,
    run: async () => ({ hash }),
  } as any;
}

describe('createQuery', () => {
  it('serves empty data immediately, then live emissions with row identity kept', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();

      expect(q.data()).toEqual([]); // committed empty, no suspension
      expect(q.isLoading()).toBe(true);

      await settle();
      eng.emit('h1', [
        { id: 'a', n: 1 },
        { id: 'b', n: 2 },
      ]);
      await settle();

      expect(q.data().map((r: any) => r.id)).toEqual(['a', 'b']);
      expect(q.isLoading()).toBe(false);
      // Rows alone do not settle a query; the server's membership does.
      expect(q.isSettled()).toBe(false);
      eng.setAuthority('h1', true);
      await settle();
      expect(q.isSettled()).toBe(true);

      const rowA = q.data()[0];
      eng.emit('h1', [
        { id: 'a', n: 99 },
        { id: 'b', n: 2 },
      ]);
      await settle();
      expect(q.data()[0]).toBe(rowA); // identity preserved via keyed reconcile
      expect(q.data()[0].n).toBe(99);

      dispose();
      await settle();
      expect(eng.unsubscribed).toContain('h1');
    });
  });

  it('an empty emission before the server answers keeps loading', async () => {
    // Nothing cached and nothing heard from the server: the only honest state
    // is "loading". A second `[]` does not change that; only authority does.
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', []);
      eng.emit('h1', []);
      await settle();
      expect(q.isLoading()).toBe(true);
      expect(q.hasData()).toBe(false);
      expect(q.isEmpty()).toBe(false);
      expect(q.isAuthoritative()).toBe(false);
      dispose();
    });
  });

  it('the server answering with no rows ends loading as empty', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', []);
      eng.setAuthority('h1', true);
      await settle();
      expect(q.isLoading()).toBe(false);
      expect(q.isEmpty()).toBe(true);
      expect(q.isAuthoritative()).toBe(true);
      expect(q.data()).toEqual([]);
      dispose();
    });
  });

  it('cached rows end loading before the server answers, without settling', async () => {
    // Local-first paint: rows from the store are shown at once. They are not
    // authoritative yet, so a windowed list must not read them as the end.
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', [{ id: 'a' }]);
      await settle();
      expect(q.isLoading()).toBe(false);
      expect(q.hasData()).toBe(true);
      expect(q.isAuthoritative()).toBe(false);
      expect(q.isSettled()).toBe(false);
      expect(q.isEmpty()).toBe(false);

      eng.setAuthority('h1', true);
      await settle();
      expect(q.isSettled()).toBe(true);
      dispose();
    });
  });

  it('a one() query the server answered as absent is empty, not loading', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, true, any>(eng.db, mockQuery('h1', true));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', []);
      eng.setAuthority('h1', true);
      await settle();
      expect(q.isLoading()).toBe(false);
      expect(q.isEmpty()).toBe(true);
      expect(q.data()).toBeNull();
      dispose();
    });
  });

  it('losing authority with no rows (a bucket switch) returns to loading', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', [{ id: 'a' }]);
      eng.setAuthority('h1', true);
      await settle();
      expect(q.isLoading()).toBe(false);

      // The rebind empties the rows and resets authority for the new principal.
      eng.emit('h1', []);
      eng.setAuthority('h1', false);
      await settle();
      expect(q.isLoading()).toBe(true);
      expect(q.isEmpty()).toBe(false);
      dispose();
    });
  });

  it('one() queries yield the row or null', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, true, any>(eng.db, mockQuery('h1', true));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      expect(q.data()).toBe(null);
      eng.emit('h1', [{ id: 'a', n: 1 }]);
      await settle();
      expect(q.data()?.n).toBe(1);
      expect(q.isLoading()).toBe(false);
      dispose();
    });
  });

  it('isFetching mirrors query status; isSettled composes both', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.setStatus('h1', 'fetching');
      flush();
      expect(q.isFetching()).toBe(true);

      eng.emit('h1', [{ id: 'a' }]);
      eng.setAuthority('h1', true);
      await settle();
      expect(q.isSettled()).toBe(false); // authoritative but still fetching

      eng.setStatus('h1', 'idle');
      flush();
      expect(q.isSettled()).toBe(true);
      dispose();
    });
  });

  it('registration failure surfaces via error(), resolving isLoading', async () => {
    const eng = mockEngine();
    const failing = {
      hash: 'hx',
      isOne: false,
      run: async () => {
        throw new Error('SSP NOT_READY');
      },
    } as any;
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, failing);
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      expect(q.error()?.message).toBe('SSP NOT_READY');
      expect(q.isLoading()).toBe(false); // spinner must resolve
      expect(q.data()).toEqual([]);
      dispose();
    });
  });

  it('reactive query thunk: identity change re-subscribes and clears state', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const [id, setId] = createSignal('h1');
      const q = createQuery<any, any, any, any, false, any>(eng.db, () => mockQuery(id()));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();

      eng.emit('h1', [{ id: 'a' }]);
      await settle();
      expect(q.data().map((r: any) => r.id)).toEqual(['a']);

      setId('h2');
      flush();
      await settle();
      await settle();

      expect(eng.unsubscribed).toContain('h1'); // superseded subscription torn down
      expect(eng.hasSub('h2')).toBe(true);
      expect(q.isLoading()).toBe(true); // reset for the new identity

      eng.emit('h2', [{ id: 'z' }]);
      await settle();
      expect(q.data().map((r: any) => r.id)).toEqual(['z']);
      dispose();
    });
  });

  it('enabled=false runs no query; flipping true starts it', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const [enabled, setEnabled] = createSignal(false);
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'), {
        enabled,
      });
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();
      expect(eng.sp00ky.subscribe).not.toHaveBeenCalled();

      setEnabled(true);
      flush();
      await settle();
      expect(eng.hasSub('h1')).toBe(true);
      dispose();
    });
  });

  it('deregisterOnCleanup deregisters the active hash on dispose', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'), {
        deregisterOnCleanup: true,
      });
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();
      expect(eng.hasSub('h1')).toBe(true);

      dispose();
      await settle();
      expect(eng.unsubscribed).toContain('h1');
      expect(eng.deregistered).toEqual(['h1']);
    });
  });

  it('reports frontend timing per emission', async () => {
    const eng = mockEngine();
    await createRoot(async (dispose) => {
      const q = createQuery<any, any, any, any, false, any>(eng.db, mockQuery('h1'));
      createEffect(
        () => q.data(),
        () => {}
      );
      flush();
      await settle();
      eng.emit('h1', [{ id: 'a' }]);
      await settle();
      expect(eng.sp00ky.reportFrontendTiming).toHaveBeenCalledWith('h1', expect.any(Number));
      dispose();
    });
  });
});
