import { createContext, useContext } from 'solid-js';
import type { SchemaStructure } from '@spooky/query-builder';
import type { SyncedDb } from '../index';

export const SpookyContext = createContext<SyncedDb<any> | undefined>();

export function useDb<S extends SchemaStructure>(): SyncedDb<S> {
  const db = useContext(SpookyContext);
  if (!db) {
    throw new Error('useDb must be used within a <SpookyProvider>. Wrap your app in <SpookyProvider config={...}>.');
  }
  return db as SyncedDb<S>;
}
