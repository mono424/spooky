import { createSignal, createEffect, onCleanup, type Accessor } from 'solid-js';
import type { SchemaStructure, BucketNames } from '@spooky-sync/query-builder';
import type { BlobUrlLease } from '@spooky-sync/core';
import type { SyncedDb } from '../index';
import { useDb } from './context';

export interface UseDownloadFileOptions {
  /**
   * Master switch, default `true`. `false` gives every hook instance its own
   * private object URL fetched fresh from the bucket and revoked on unmount —
   * no sharing, no persistence, no reuse.
   */
  cache?: boolean;
  /**
   * Keep the bytes in OPFS so they survive a reload and are available offline.
   * Default `true`. Turn off for one-shot or sensitive files; the in-tab object
   * URL is still shared between components rendering the same path.
   */
  persist?: boolean;
  /** Exempt this file from pressure eviction. Pinned bytes never expire. */
  pin?: boolean;
  /**
   * `'never'` (default) treats a bucket path as immutable, which is how paths
   * are written (`crypto.randomUUID() + ext`). `'head'` spends a remote `head()`
   * to compare sizes before trusting the cached copy — for paths the app
   * overwrites in place.
   */
  revalidate?: 'never' | 'head';
}

export interface UseDownloadFileResult {
  url: Accessor<string | null>;
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  refetch: () => void;
}

export function useDownloadFile<S extends SchemaStructure>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions
): UseDownloadFileResult;
export function useDownloadFile<S extends SchemaStructure>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions
): UseDownloadFileResult;
export function useDownloadFile<S extends SchemaStructure>(
  dbOrBucketName: SyncedDb<S> | BucketNames<S>,
  bucketNameOrPath?: BucketNames<S> | Accessor<string | null | undefined>,
  pathOrOptions?: Accessor<string | null | undefined> | UseDownloadFileOptions,
  maybeOptions?: UseDownloadFileOptions
): UseDownloadFileResult {
  let db: SyncedDb<S>;
  let bucketName: BucketNames<S>;
  let path: Accessor<string | null | undefined>;
  let options: UseDownloadFileOptions;

  if (typeof dbOrBucketName === 'string') {
    db = useDb<S>();
    bucketName = dbOrBucketName as BucketNames<S>;
    path = bucketNameOrPath as Accessor<string | null | undefined>;
    options = (pathOrOptions as UseDownloadFileOptions) ?? {};
  } else {
    db = dbOrBucketName as SyncedDb<S>;
    bucketName = bucketNameOrPath as BucketNames<S>;
    path = pathOrOptions as Accessor<string | null | undefined>;
    options = maybeOptions ?? {};
  }

  const useCache = options.cache !== false;

  // Written from fetch continuations — outside any tracking scope.
  const [url, setUrl] = createSignal<string | null>(null, { ownedWrite: true });
  const [isLoading, setIsLoading] = createSignal(false, { ownedWrite: true });
  const [error, setError] = createSignal<Error | null>(null, { ownedWrite: true });

  // Exactly one of these is held at a time: a refcounted lease on the shared
  // cache entry, or a private URL this instance minted and must revoke itself.
  let lease: BlobUrlLease | null = null;
  let privateUrl: string | null = null;

  const [refetchSignal, setRefetchSignal] = createSignal(0);
  /** Consumed by the next effect run, so `refetch()` bypasses every layer once. */
  let reloadOnce = false;

  function releaseCurrent() {
    lease?.release();
    lease = null;
    if (privateUrl) {
      URL.revokeObjectURL(privateUrl);
      privateUrl = null;
    }
  }

  // Two-arg Solid 2 effect: compute tracks path + refetch tick; apply runs the
  // fetch and returns the cancel/release cleanup, which runs before the next
  // apply and on unmount.
  createEffect(
    () => {
      refetchSignal();
      return path();
    },
    (filePath) => {
      releaseCurrent();

      if (!filePath) {
        setUrl(null);
        setIsLoading(false);
        setError(null);
        return;
      }

      const reload = reloadOnce;
      reloadOnce = false;

      let cancelled = false;
      setIsLoading(true);
      setError(null);

      const bucket = db.bucket(bucketName);
      const resolve = useCache
        ? bucket
            .url(filePath, {
              persist: options.persist !== false,
              pin: options.pin,
              revalidate: options.revalidate,
              reload,
            })
            .then((acquired) => {
              if (!acquired) return null;
              if (cancelled) {
                // Unmounted or the path changed mid-flight — hand the reference
                // straight back, or the entry never drops to zero and its object
                // URL leaks for the life of the tab.
                acquired.release();
                return null;
              }
              lease = acquired;
              return acquired.url;
            })
        : bucket.read(filePath, { persist: false, reload: true }).then((blob) => {
            if (!blob || cancelled) return null;
            privateUrl = URL.createObjectURL(blob);
            return privateUrl;
          });

      resolve.then(
        (result) => {
          if (!cancelled) {
            setUrl(result);
            setIsLoading(false);
          }
          return undefined;
        },
        (err) => {
          if (!cancelled) {
            setError(err instanceof Error ? err : new Error(String(err)));
            setIsLoading(false);
          }
        }
      );

      return () => {
        cancelled = true;
      };
    }
  );

  onCleanup(() => {
    releaseCurrent();
  });

  const refetch = () => {
    reloadOnce = true;
    setRefetchSignal((n) => n + 1);
  };

  return { url, isLoading, error, refetch };
}
