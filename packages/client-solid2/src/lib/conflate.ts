/**
 * Latest-wins async iterable over a subscribe-callback source.
 *
 * Bridges spooky's push-callback subscriptions into the AsyncIterable shape
 * Solid 2 computations consume natively. Each spooky emission is a full result
 * set, so intermediate values are droppable: only the newest unconsumed value
 * is buffered, and a pending pull resolves with it immediately.
 *
 * Teardown contract (probed in rc-semantics.test.ts): Solid 2 does NOT
 * terminate a superseded/disposed computation's async generator — no
 * `return()`, no `finally`. Consumers MUST call `it.return()` themselves from
 * an `onCleanup` registered synchronously in the compute scope. `return()`
 * unsubscribes (awaiting the unsubscribe if the subscribe returned a promise,
 * as `sp00ky.subscribe` does) and resolves any parked pull as done.
 */
export function conflate<T>(
  subscribe: (cb: (v: T) => void) => (() => void) | Promise<() => void>
): AsyncIterable<T> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<T> {
      let buffered: { v: T } | undefined;
      let resolveNext: ((r: IteratorResult<T>) => void) | undefined;
      let done = false;

      const unsubMaybe = subscribe((v) => {
        if (done) return;
        if (resolveNext) {
          const r = resolveNext;
          resolveNext = undefined;
          r({ value: v, done: false });
        } else {
          buffered = { v };
        }
      });

      const finish = () => {
        if (done) return;
        done = true;
        buffered = undefined;
        // Unsubscribe may still be in flight (async registration); chain it.
        Promise.resolve(unsubMaybe)
          .then((unsub) => unsub())
          .catch(() => {
            // Registration failed — there is nothing to unsubscribe.
          });
        if (resolveNext) {
          const r = resolveNext;
          resolveNext = undefined;
          r({ value: undefined as never, done: true });
        }
      };

      return {
        next(): Promise<IteratorResult<T>> {
          if (done) return Promise.resolve({ value: undefined as never, done: true });
          if (buffered) {
            const v = buffered.v;
            buffered = undefined;
            return Promise.resolve({ value: v, done: false });
          }
          return new Promise<IteratorResult<T>>((r) => (resolveNext = r));
        },
        return(): Promise<IteratorResult<T>> {
          finish();
          return Promise.resolve({ value: undefined as never, done: true });
        },
        throw(e: unknown): Promise<IteratorResult<T>> {
          finish();
          return Promise.reject(e);
        },
      };
    },
  };
}
