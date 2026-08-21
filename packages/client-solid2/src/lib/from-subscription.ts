import { createMemo, onCleanup, type Accessor } from 'solid-js';
import { conflate } from './conflate';

/**
 * Reactive view over a spooky subscribe-callback API.
 *
 * The memo's async generator pulls from a conflated (latest-wins) iterator;
 * `initial` is committed as the memo's `loadingValue`, so the accessor is
 * readable synchronously from birth and never suspends. Spooky's subscribe
 * APIs fire immediately with the current value, so the real value lands within
 * a tick of the first read.
 *
 * Teardown is manual by contract (see conflate.ts): onCleanup terminates the
 * iterator, which unsubscribes.
 */
export function fromSubscription<T>(
  subscribe: (cb: (v: T) => void) => (() => void) | Promise<() => void>,
  initial: T
): Accessor<T> {
  return createMemo(
    async function* (): AsyncGenerator<T> {
      const it = conflate(subscribe)[Symbol.asyncIterator]();
      onCleanup(() => void it.return?.());
      while (true) {
        const r = await it.next();
        if (r.done) break;
        yield r.value;
      }
    },
    { loadingValue: initial }
  );
}
