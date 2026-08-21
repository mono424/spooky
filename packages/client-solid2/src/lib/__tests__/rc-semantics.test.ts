/**
 * Probes against solid-js 2.0.0-rc.0 semantics this package's design depends
 * on. If any of these fail after a Solid version bump, the binding's
 * assumptions are broken — fix the binding, don't delete the probe.
 *
 * Probed assumptions (see plan):
 * 1. A projection compute given as an async generator restarts when a
 *    dependency read before the first `await` changes, and the superseded
 *    generator is terminated (its `finally` runs) so external subscriptions
 *    can be torn down.
 * 2. Keyed reconcile of yielded arrays notifies coarse readers (whole-array
 *    tracking, e.g. `<For>`/mapArray) on add/remove even when the array length
 *    stays equal (windowed-list delete case).
 * 3. `seedLoadingValue: true` births the store committed: readable
 *    immediately, `isPending` false during the first flight.
 * 4. Signals created with `{ ownedWrite: true }` accept writes from plain
 *    callbacks fired outside any tracking scope after setup.
 * 5. Class instances (RecordId-shaped) inside store rows: document whether the
 *    proxy wraps them and that instanceof/method access still works through it.
 * 6. Generator teardown on owner dispose runs `finally` blocks.
 */
import { describe, expect, it } from 'vitest';
import {
  createProjection,
  createRoot,
  createSignal,
  createEffect,
  createMemo,
  isPending,
  flush,
  mapArray,
  isWrappable,
  snapshot,
  onCleanup,
} from 'solid-js';
import * as signals from '@solidjs/signals';

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

/** Minimal push-source with subscriber tracking, stand-in for sp00ky.subscribe. */
function pushSource<T>() {
  let cb: ((v: T) => void) | undefined;
  let subscribes = 0;
  let unsubscribes = 0;
  return {
    subscribe(fn: (v: T) => void) {
      cb = fn;
      subscribes++;
      return () => {
        if (cb === fn) cb = undefined;
        unsubscribes++;
      };
    },
    emit(v: T) {
      cb?.(v);
    },
    get counts() {
      return { subscribes, unsubscribes };
    },
  };
}

/** Push → pull adapter (same shape as ../conflate, inlined to keep the probe
 *  self-contained). */
function iterate<T>(subscribe: (cb: (v: T) => void) => () => void): AsyncIterable<T> {
  return {
    [Symbol.asyncIterator]() {
      let buffered: { v: T } | undefined;
      let resolveNext: ((r: IteratorResult<T>) => void) | undefined;
      let done = false;
      const unsub = subscribe((v) => {
        if (done) return;
        if (resolveNext) {
          const r = resolveNext;
          resolveNext = undefined;
          r({ value: v, done: false });
        } else {
          buffered = { v };
        }
      });
      const finish = (): IteratorResult<T> => {
        if (!done) {
          done = true;
          unsub();
          resolveNext?.({ value: undefined, done: true });
          resolveNext = undefined;
        }
        return { value: undefined, done: true };
      };
      return {
        next() {
          if (done) return Promise.resolve<IteratorResult<T>>({ value: undefined, done: true });
          if (buffered) {
            const v = buffered.v;
            buffered = undefined;
            return Promise.resolve({ value: v, done: false });
          }
          return new Promise<IteratorResult<T>>((r) => (resolveNext = r));
        },
        return() {
          return Promise.resolve(finish());
        },
        throw(e: unknown) {
          finish();
          return Promise.reject(e);
        },
      };
    },
  };
}

type Row = { id: string; n?: number };

describe('probe 1: async-generator projection restart on dep change', () => {
  it('abandons the superseded generator: teardown must be manual via onCleanup', async () => {
    // rc.0 semantics: a dep change restarts the compute (a new generator
    // starts) but the superseded generator is NOT terminated — no return(),
    // no finally. Anything it subscribed to leaks unless the compute registers
    // onCleanup (synchronously, before the first await) to tear it down.
    // conflate() + create-query rely on exactly that onCleanup contract.
    const src1 = pushSource<Row[]>();
    const src2 = pushSource<Row[]>();
    const finished: string[] = [];

    await createRoot(async (dispose) => {
      const [which, setWhich] = createSignal<'a' | 'b'>('a');

      const rows = createProjection(
        async function* (): AsyncGenerator<Row[]> {
          const w = which(); // tracked read BEFORE first await
          const src = w === 'a' ? src1 : src2;
          const it = iterate<Row[]>((cb) => src.subscribe(cb))[Symbol.asyncIterator]();
          onCleanup(() => void it.return?.()); // manual teardown — Solid won't do it
          try {
            while (true) {
              const r = await it.next();
              if (r.done) break;
              yield r.value;
            }
          } finally {
            finished.push(w);
          }
        },
        [] as Row[],
        { key: 'id' }
      );

      // touch the projection so it computes
      createEffect(
        () => rows.length,
        () => {}
      );
      flush();
      await tick();
      expect(src1.counts.subscribes).toBe(1);

      src1.emit([{ id: 'x', n: 1 }]);
      await tick();
      flush();
      expect(rows.map((r) => r.id)).toEqual(['x']);

      // dep change → onCleanup fires for generator A, its finally runs, B subscribes
      setWhich('b');
      flush();
      await tick();
      await tick();
      expect(finished).toContain('a');
      expect(src1.counts.unsubscribes).toBe(1);
      expect(src2.counts.subscribes).toBe(1);

      src2.emit([{ id: 'y', n: 2 }]);
      await tick();
      flush();
      expect(rows.map((r) => r.id)).toEqual(['y']);

      dispose();
      await tick();
      // dispose runs the live generator's onCleanup too
      expect(finished).toContain('b');
      expect(src2.counts.unsubscribes).toBe(1);
    });
  });
});

describe('probe 2: keyed reconcile notifies coarse readers on same-length change', () => {
  it('mapArray over the projection re-runs when a row is swapped', async () => {
    const src = pushSource<Row[]>();
    await createRoot(async (dispose) => {
      const rows = createProjection(
        async function* (): AsyncGenerator<Row[]> {
          for await (const v of iterate<Row[]>((cb) => src.subscribe(cb))) yield v;
        },
        [] as Row[],
        { key: 'id' }
      );

      const seen: string[][] = [];
      const mapped = mapArray(
        () => rows,
        (r) => r.id
      );
      createEffect(
        () => mapped(),
        (ids) => {
          seen.push([...ids]);
        }
      );
      flush();
      await tick();

      src.emit([
        { id: 'a', n: 1 },
        { id: 'b', n: 2 },
      ]);
      await tick();
      flush();

      // same length, one row swapped (windowed-list delete: c shifts in for b)
      src.emit([
        { id: 'a', n: 1 },
        { id: 'c', n: 3 },
      ]);
      await tick();
      flush();

      expect(seen.at(-1)).toEqual(['a', 'c']);

      // row identity for the surviving row must be stable across emissions
      const a1 = rows[0];
      src.emit([
        { id: 'a', n: 99 },
        { id: 'c', n: 3 },
      ]);
      await tick();
      flush();
      expect(rows[0]).toBe(a1);
      expect(rows[0].n).toBe(99);

      dispose();
    });
  });
});

describe('probe 3: seedLoadingValue commits the seed', () => {
  it('store is readable and not pending during first flight', async () => {
    const src = pushSource<Row[]>();
    await createRoot(async (dispose) => {
      const rows = createProjection(
        async function* (): AsyncGenerator<Row[]> {
          for await (const v of iterate<Row[]>((cb) => src.subscribe(cb))) yield v;
        },
        [] as Row[],
        { key: 'id', seedLoadingValue: true }
      );

      let pendingDuringFlight: boolean | undefined;
      let lenDuringFlight: number | undefined;
      createEffect(
        () => {
          lenDuringFlight = rows.length;
          pendingDuringFlight = isPending(() => rows.length);
          return undefined;
        },
        () => {}
      );
      flush();
      await tick();

      expect(lenDuringFlight).toBe(0); // readable, no NotReadyError
      expect(pendingDuringFlight).toBe(false); // commit #0, not pending

      src.emit([{ id: 'a' }]);
      await tick();
      flush();
      expect(rows.length).toBe(1);
      dispose();
    });
  });
});

describe('probe 4: ownedWrite signals accept external-callback writes', () => {
  it('does not throw when written from a later plain callback', async () => {
    let later: (() => void) | undefined;
    await createRoot(async (dispose) => {
      const [v, setV] = createSignal(0, { ownedWrite: true });
      later = () => setV(1);
      createEffect(
        () => v(),
        () => {}
      );
      flush();
      // fire from outside any reactive scope, as a subscription callback would
      await tick();
      expect(() => later!()).not.toThrow();
      flush();
      expect(v()).toBe(1);
      dispose();
    });
  });
});

describe('probe 5: class instances inside store rows', () => {
  class FakeRecordId {
    constructor(
      public tb: string,
      public id: string
    ) {}
    toString() {
      return `${this.tb}:${this.id}`;
    }
  }

  it('documents wrapping behavior and that instanceof/methods survive', async () => {
    const src = pushSource<{ id: string; rid: FakeRecordId }[]>();
    await createRoot(async (dispose) => {
      const rows = createProjection(
        async function* (): AsyncGenerator<{ id: string; rid: FakeRecordId }[]> {
          for await (const v of iterate<{ id: string; rid: FakeRecordId }[]>((cb) =>
            src.subscribe(cb)
          ))
            yield v;
        },
        [] as { id: string; rid: FakeRecordId }[],
        { key: 'id' }
      );
      createEffect(
        () => rows.length,
        () => {}
      );
      flush();
      await tick();

      src.emit([{ id: 'a', rid: new FakeRecordId('game', 'a') }]);
      await tick();
      flush();

      const rid = rows[0].rid;
      // Solid 2 wraps class instances (isWrappable true — unlike Solid 1).
      expect(isWrappable(new FakeRecordId('t', 'i'))).toBe(true);
      // Document what survives through the proxy: methods are served BOUND, so
      // `constructor.name` gains a 'bound ' prefix. SyncedDb.delete's
      // cross-package RecordId detection must strip it.
      expect((rid as any).constructor?.name).toBe('bound FakeRecordId');
      expect(rid.toString()).toBe('game:a');
      expect(`${rid.tb}:${rid.id}`).toBe('game:a');
      // `snapshot` must unwrap back to the raw instance so payloads passed to
      // surrealdb (which checks instanceof) can be de-proxied at the boundary.
      const snap = snapshot(rows[0]);
      expect(snap.rid instanceof FakeRecordId).toBe(true);
      dispose();
    });
  });

  it('rc.0 packaging: markRaw is typed but absent from the runtime build', () => {
    // store.d.ts declares markRaw, but dist/dev.js does not export it. If this
    // starts passing after a bump, adopt markRaw for RecordId/CrdtField
    // instances at the ingest boundary and drop the snapshot() workaround.
    expect((signals as any).markRaw).toBeUndefined();
  });
});

describe('probe: async-generator memo (fromSubscription shape)', () => {
  it('memo over async generator with loadingValue serves values', async () => {
    const src = pushSource<number>();
    await createRoot(async (dispose) => {
      const v = createMemo(
        async function* (): AsyncGenerator<number> {
          for await (const x of iterate<number>((cb) => src.subscribe(cb))) yield x;
        },
        { loadingValue: -1 }
      );
      let latest: number | undefined;
      createEffect(
        () => v(),
        (x) => {
          latest = x;
        }
      );
      flush();
      await tick();
      expect(latest).toBe(-1); // loadingValue committed
      src.emit(42);
      await tick();
      flush();
      expect(latest).toBe(42);
      dispose();
    });
  });
});
