import { createSignal, onCleanup, type Accessor } from 'solid-js';
import { useDb } from './context';
import type { SyncHealth, SyncHealthStatus } from '@spooky-sync/core';

export interface UseSyncStatus {
  /** Full health snapshot; updates reactively on every transition. */
  health: Accessor<SyncHealth>;
  /** `'healthy'` | `'degraded'`. */
  status: Accessor<SyncHealthStatus>;
  isHealthy: Accessor<boolean>;
  /** `true` once sync has failed for a sustained run — drive a banner off this. */
  isDegraded: Accessor<boolean>;
  /** `true` once at least one sync round has succeeded this session. */
  everConnected: Accessor<boolean>;
  /**
   * `true` only for a real lost connection: degraded AFTER a first successful
   * sync. Stays `false` during the initial "connecting" phase (degraded but
   * never reached the server yet), so an indicator can show nothing until the
   * app has actually connected once.
   */
  isOffline: Accessor<boolean>;
}

/**
 * Observe sync health for a "can't reach the server" banner / indicator.
 *
 * Backed by `db.subscribeToSyncHealth`. Individual sync failures (a transient
 * remote 500 on query registration, a dropped socket) are absorbed by the
 * retry and never flip this; `isDegraded()` only goes true once failures
 * persist for the configured number of consecutive rounds (sp00ky core config
 * `syncHealth.degradeAfterConsecutiveFailures`, default 3), and flips back on
 * the next successful round. Must be used within a `<Sp00kyProvider>`.
 */
export function useSyncStatus(): UseSyncStatus {
  const db = useDb();
  // subscribeToSyncHealth fires synchronously with the current status, so the
  // signal is correct from first read; the initial value just avoids a flash.
  const [health, setHealth] = createSignal<SyncHealth>(db.syncHealth);
  const unsub = db.subscribeToSyncHealth(setHealth);
  onCleanup(unsub);

  return {
    health,
    status: () => health().status,
    isHealthy: () => health().status === 'healthy',
    isDegraded: () => health().status === 'degraded',
    everConnected: () => health().everConnected,
    isOffline: () => health().status === 'degraded' && health().everConnected,
  };
}
