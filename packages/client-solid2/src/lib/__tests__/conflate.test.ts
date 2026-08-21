import { describe, expect, it } from 'vitest';
import { createEffect, createRoot, flush } from 'solid-js';
import { conflate } from '../conflate';
import { fromSubscription } from '../from-subscription';

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe('conflate', () => {
  it('delivers values pushed after a pull is parked', async () => {
    let cb: ((v: number) => void) | undefined;
    const it = conflate<number>((c) => {
      cb = c;
      return () => (cb = undefined);
    })[Symbol.asyncIterator]();

    const p = it.next();
    cb!(1);
    expect(await p).toEqual({ value: 1, done: false });
  });

  it('conflates: only the newest unconsumed value survives', async () => {
    let cb: ((v: number) => void) | undefined;
    const it = conflate<number>((c) => {
      cb = c;
      return () => (cb = undefined);
    })[Symbol.asyncIterator]();

    cb!(1);
    cb!(2);
    cb!(3);
    expect(await it.next()).toEqual({ value: 3, done: false });
  });

  it('return() unsubscribes and resolves a parked pull as done', async () => {
    let unsubscribed = false;
    let cb: ((v: number) => void) | undefined;
    const it = conflate<number>((c) => {
      cb = c;
      return () => {
        unsubscribed = true;
        cb = undefined;
      };
    })[Symbol.asyncIterator]();

    const parked = it.next();
    await it.return!();
    await tick();
    expect(unsubscribed).toBe(true);
    expect(await parked).toEqual({ value: undefined, done: true });
    expect(await it.next()).toEqual({ value: undefined, done: true });
  });

  it('supports async subscribe (unsubscribe still runs after return())', async () => {
    let unsubscribed = false;
    const it = conflate<number>(async () => {
      await tick();
      return () => {
        unsubscribed = true;
      };
    })[Symbol.asyncIterator]();

    await it.return!();
    await tick();
    await tick();
    expect(unsubscribed).toBe(true);
  });

  it('values pushed after return() are dropped', async () => {
    let cb: ((v: number) => void) | undefined;
    const it = conflate<number>((c) => {
      cb = c;
      return () => {};
    })[Symbol.asyncIterator]();
    await it.return!();
    cb!(42);
    expect(await it.next()).toEqual({ value: undefined, done: true });
  });
});

describe('fromSubscription', () => {
  it('serves initial synchronously, then live values; unsubscribes on dispose', async () => {
    let cb: ((v: number) => void) | undefined;
    let unsubscribed = false;
    const subscribe = (c: (v: number) => void) => {
      cb = c;
      // spooky-style: fire immediately with current value
      c(10);
      return () => {
        unsubscribed = true;
        cb = undefined;
      };
    };

    await createRoot(async (dispose) => {
      const v = fromSubscription(subscribe, -1);
      const seen: number[] = [];
      createEffect(
        () => v(),
        (x) => {
          seen.push(x);
        }
      );
      flush();
      expect(v()).toBe(-1); // loadingValue readable synchronously
      await tick();
      flush();
      expect(v()).toBe(10); // immediate emission landed

      cb!(20);
      await tick();
      flush();
      expect(v()).toBe(20);
      expect(seen).toContain(20);

      dispose();
      await tick();
      expect(unsubscribed).toBe(true);
    });
  });
});
