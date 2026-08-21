import { createSignal, type Accessor } from 'solid-js';

export interface Submission<Args extends unknown[], R> {
  /** Run the wrapped async fn. Concurrent submits share the pending flag. */
  submit: (...args: Args) => Promise<R | undefined>;
  /** True while at least one submit is in flight. */
  pending: Accessor<boolean>;
  /** Error from the most recent settled submit, cleared on the next submit. */
  error: Accessor<Error | undefined>;
  /** Result of the most recent successful submit. */
  result: Accessor<R | undefined>;
  clearError: () => void;
}

/**
 * Thin submission-state wrapper for mutations — button spinner/disable state
 * around `db.create/update/delete/run` calls.
 *
 * Deliberately NOT built on Solid 2's `action()`/`createOptimisticStore`: the
 * spooky engine is already optimistic local-first (writes commit to the local
 * DB and re-render through live queries before sync; `run()` is an outbox
 * CREATE), so a transaction/revert layer on top buys nothing and `action()`'s
 * await-vs-yield transaction escape is a real footgun. Errors here mean the
 * LOCAL commit failed — sync/push failures surface through `useSyncStatus`
 * and `usePendingMutations` instead.
 */
export function createSubmission<Args extends unknown[], R>(
  fn: (...args: Args) => Promise<R>
): Submission<Args, R> {
  // Written from promise continuations — outside any tracking scope.
  const [inFlight, setInFlight] = createSignal(0, { ownedWrite: true });
  const [error, setError] = createSignal<Error | undefined>(undefined, { ownedWrite: true });
  const [result, setResult] = createSignal<R | undefined>(undefined, { ownedWrite: true });

  const submit = async (...args: Args): Promise<R | undefined> => {
    setError(undefined);
    setInFlight((n) => n + 1);
    try {
      const r = await fn(...args);
      setResult(() => r);
      return r;
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
      return undefined;
    } finally {
      setInFlight((n) => n - 1);
    }
  };

  return {
    submit,
    pending: () => inFlight() > 0,
    error,
    result,
    clearError: () => setError(undefined),
  };
}
