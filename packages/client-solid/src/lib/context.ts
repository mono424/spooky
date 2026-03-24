import { createContext, useContext } from 'solid-js';
import type { SchemaStructure } from '@spooky/query-builder';
import type { SyncedDb } from '../index';

export const Sp00kyContext = createContext<SyncedDb<any> | undefined>();

export function useDb<S extends SchemaStructure>(): SyncedDb<S> {
  const db = useContext(Sp00kyContext);
  if (!db) {
    throw new Error('useDb must be used within a <Sp00kyProvider>. Wrap your app in <Sp00kyProvider config={...}>.');
  }
  return db as SyncedDb<S>;
}
