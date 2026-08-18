import { createSignal, createEffect, onCleanup, type Accessor } from 'solid-js';
import type { SchemaStructure, BucketNames } from '@spooky-sync/query-builder';
import type { SyncedDb } from '../index';
import { useDb } from './context';

export interface UseBlurhashResult {
  /** The stored blurhash for the path, or null while loading / when none exists. */
  hash: Accessor<string | null>;
  isLoading: Accessor<boolean>;
}

/**
 * The blurhash sidecar for a bucket image (written automatically by
 * `bucket.put`, see `Sp00kyConfig.blurhash`). Resolves from OPFS instantly on
 * warm clients; a miss is remembered per tab. Use this directly when the hash
 * belongs to a different rendition than the displayed image; otherwise
 * `useBucketImage` / `BucketImage` bundle it with the download.
 */
export function useBlurhash<S extends SchemaStructure>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>
): UseBlurhashResult;
export function useBlurhash<S extends SchemaStructure>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>
): UseBlurhashResult;
export function useBlurhash<S extends SchemaStructure>(
  dbOrBucketName: SyncedDb<S> | BucketNames<S>,
  bucketNameOrPath?: BucketNames<S> | Accessor<string | null | undefined>,
  maybePath?: Accessor<string | null | undefined>
): UseBlurhashResult {
  let db: SyncedDb<S>;
  let bucketName: BucketNames<S>;
  let path: Accessor<string | null | undefined>;

  if (typeof dbOrBucketName === 'string') {
    db = useDb<S>();
    bucketName = dbOrBucketName as BucketNames<S>;
    path = bucketNameOrPath as Accessor<string | null | undefined>;
  } else {
    db = dbOrBucketName as SyncedDb<S>;
    bucketName = bucketNameOrPath as BucketNames<S>;
    path = maybePath as Accessor<string | null | undefined>;
  }

  const [hash, setHash] = createSignal<string | null>(null);
  const [isLoading, setIsLoading] = createSignal(false);

  createEffect(() => {
    const filePath = path();
    if (!filePath) {
      setHash(null);
      setIsLoading(false);
      return;
    }
    let cancelled = false;
    setIsLoading(true);
    db.bucket(bucketName)
      .blurhash(filePath)
      .then((result) => {
        if (cancelled) return;
        setHash(result);
        setIsLoading(false);
      })
      .catch(() => {
        if (cancelled) return;
        setHash(null);
        setIsLoading(false);
      });
    onCleanup(() => {
      cancelled = true;
    });
  });

  return { hash, isLoading };
}
