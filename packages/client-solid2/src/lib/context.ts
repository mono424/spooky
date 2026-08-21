import { createContext, useContext, type Accessor } from 'solid-js';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { SyncedDb } from '../index';
import { fromSubscription } from './from-subscription';

// Solid 2: the context object doubles as its provider component —
// <Sp00kyContext value={db}>{children}</Sp00kyContext>.
export const Sp00kyContext = createContext<SyncedDb<any>>();

export function useDb<S extends SchemaStructure>(): SyncedDb<S> {
  try {
    return useContext(Sp00kyContext) as SyncedDb<S>;
  } catch {
    // Solid 2 throws ContextNotFoundError; rethrow with actionable guidance.
    throw new Error(
      'useDb must be used within a <Sp00kyProvider>. Wrap your app in <Sp00kyProvider config={...}>.'
    );
  }
}

/**
 * Count of locally-committed mutations not yet acknowledged by the server.
 * Drive an "unsaved changes" indicator off this.
 */
export function usePendingMutations(): Accessor<number> {
  const db = useDb();
  return fromSubscription((cb) => db.subscribeToPendingMutations(cb), db.pendingMutationCount);
}
