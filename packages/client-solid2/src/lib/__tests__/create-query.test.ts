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
  const unsubscribed: string[] = [];
  const deregistered: string[] = [];
  const sp00ky = {
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

  it('first empty emission does not mark fetched; second does', async () => {
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
      await settle();
      expect(q.isLoading()).toBe(true); // still loading: local DB likely not synced

      eng.emit('h1', []);
      await settle();
      expect(q.isLoading()).toBe(false); // a later empty emission is authoritative
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
      await settle();
      expect(q.isSettled()).toBe(false); // fetched but still fetching

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
