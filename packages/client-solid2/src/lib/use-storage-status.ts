import type { Accessor } from 'solid-js';
import { useDb } from './context';
import { fromSubscription } from './from-subscription';
import type { StorageHealth, StorageHealthStatus } from '@spooky-sync/core';

export interface UseStorageStatus {
  /** Full durability snapshot; updates reactively. */
  health: Accessor<StorageHealth>;
  /** `'unknown'` | `'persistent'` | `'memory'`. */
  status: Accessor<StorageHealthStatus>;
  /** `true` when the local store survives a reload. */
  isPersistent: Accessor<boolean>;
  /**
   * `true` only when durable storage was requested and could NOT be opened, so
   * the dataset is sitting in RAM and local writes die on reload. Drive a
   * warning off this, not off `status`: a store configured as in-memory reports
   * `'memory'` too, and that is a choice rather than a problem.
   */
  isMemoryFallback: Accessor<boolean>;
}

/**
 * Observe how durable the LOCAL cache is, for a "no local storage" warning.
 *
 * Under `localEngine: 'sqlite'` with `store: 'indexeddb'` the durable store is
 * the OPFS SAHPool VFS, and only ONE client per bucket can hold it open: a
 * second tab of the same app cannot get it and runs in memory instead (the
 * engine retries first, so a closing tab's lock is usually waited out). Must be
 * used within a `<Sp00kyProvider>`.
 */
export function useStorageStatus(): UseStorageStatus {
  const db = useDb();
  const health = fromSubscription<StorageHealth>(
    (cb) => db.subscribeToStorageHealth(cb),
    db.storageHealth
  );

  return {
    health,
    status: () => health().status,
    isPersistent: () => health().status === 'persistent',
    isMemoryFallback: () => health().fallback,
  };
}
