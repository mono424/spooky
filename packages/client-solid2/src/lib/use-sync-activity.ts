import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { useDb } from './context';
import { fromSubscription } from './from-subscription';
import type { SyncedDb } from '../index';
import type { SchemaStructure } from '@spooky-sync/query-builder';

export interface UseSyncActivityOptions {
  /**
   * How long queries must be fetching before `isDownloading()` turns on.
   * Filters the sub-frame fetches a warm cache produces, so the indicator only
   * shows work the user could notice. Default 200 ms.
   */
  downloadDelayMs?: number;
  /**
   * `isUploading()` turns on once MORE than this many mutations are waiting in
   * the outbox. Default 1: a single write acknowledged within a round trip is
   * not worth an animation; a backlog is.
   */
  uploadThreshold?: number;
}

export interface UseSyncActivity {
  /** Queries inside a fetch cycle right now (core `fetchingQueryCount`). */
  fetchingQueries: Accessor<number>;
  /** Locally committed writes the server has not acknowledged yet. */
  pendingMutations: Accessor<number>;
  /** Fetching for longer than `downloadDelayMs`. Drives a "downloading" mark. */
  isDownloading: Accessor<boolean>;
  /** More than `uploadThreshold` writes queued. Drives an "uploading" mark. */
  isUploading: Accessor<boolean>;
}

/**
 * The two directions of sync traffic, for an indicator in the app chrome.
 *
 * `fetchingQueries` is one subscription on the engine's aggregate fetch count,
 * not one per query; `pendingMutations` is the outbox depth. `isDownloading`
 * is the fetch count debounced ON by `downloadDelayMs` (and off at once), so a
 * local-first page that answers from cache and confirms with the server in a
 * few milliseconds never flickers. Must be used within a `<Sp00kyProvider>`, or
 * pass the `SyncedDb` explicitly.
 */
export function useSyncActivity<S extends SchemaStructure = any>(
  dbOrOptions?: SyncedDb<S> | UseSyncActivityOptions,
  maybeOptions?: UseSyncActivityOptions
): UseSyncActivity {
  const explicitDb =
    dbOrOptions && typeof (dbOrOptions as SyncedDb<S>).subscribeToFetchActivity === 'function'
      ? (dbOrOptions as SyncedDb<S>)
      : undefined;
  const options = (explicitDb ? maybeOptions : (dbOrOptions as UseSyncActivityOptions)) ?? {};
  const db = explicitDb ?? useDb<S>();
  const downloadDelayMs = options.downloadDelayMs ?? 200;
  const uploadThreshold = options.uploadThreshold ?? 1;

  const fetchingQueries = fromSubscription<number>(
    (cb) => db.subscribeToFetchActivity(cb),
    db.fetchingQueryCount
  );
  const pendingMutations = fromSubscription<number>(
    (cb) => db.subscribeToPendingMutations(cb),
    db.pendingMutationCount
  );

  // ON after the delay, OFF immediately. Written from a timer, hence ownedWrite.
  const [isDownloading, setIsDownloading] = createSignal(false, { ownedWrite: true });
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    () => fetchingQueries() > 0,
    (busy) => {
      if (timer !== undefined) {
        clearTimeout(timer);
        timer = undefined;
      }
      if (!busy) {
        setIsDownloading(false);
        return;
      }
      if (downloadDelayMs <= 0) {
        setIsDownloading(true);
        return;
      }
      timer = setTimeout(() => {
        timer = undefined;
        setIsDownloading(true);
      }, downloadDelayMs);
    }
  );
  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
  });

  return {
    fetchingQueries,
    pendingMutations,
    isDownloading,
    isUploading: () => pendingMutations() > uploadThreshold,
  };
}
