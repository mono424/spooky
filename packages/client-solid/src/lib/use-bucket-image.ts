import { createSignal, createEffect, on, type Accessor } from 'solid-js';
import type { SchemaStructure, BucketNames } from '@spooky-sync/query-builder';
import type { SyncedDb } from '../index';
import { useDb } from './context';
import {
  useDownloadFile,
  type UseDownloadFileOptions,
  type UseDownloadFileResult,
} from './use-download-file';
import { useBlurhash } from './use-blurhash';

export interface UseBucketImageOptions extends UseDownloadFileOptions {
  /**
   * Also resolve the image's blurhash sidecar (see `Sp00kyConfig.blurhash`).
   * Default `true`; the read is registered before the image bytes so the tiny
   * sidecar tends to land first on the serialized remote chain.
   */
  blurhash?: boolean;
}

export interface UseBucketImageResult extends UseDownloadFileResult {
  /** Blurhash for the same path, or null (off, missing, still loading). */
  blurhash: Accessor<string | null>;
  /** True once the current `url()` has been decoded and is safe to paint. */
  ready: Accessor<boolean>;
  /**
   * Ref callback for the `<img>` rendering `url()`: flips `ready` when the
   * bitmap is decoded (resolves on failure too, so a broken blob degrades to
   * paint-on-load instead of hiding the image forever). Re-arms itself when
   * the url changes.
   */
  gate: (img: HTMLImageElement) => void;
}

/**
 * Everything needed to render a bucket image without a pop-in: the refcounted
 * object URL, the blurhash placeholder, and a decode gate so the real bitmap
 * is only revealed once it can paint in full. `BucketImage` wraps this into a
 * drop-in component; use the hook for custom markup.
 */
export function useBucketImage<S extends SchemaStructure>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseBucketImageOptions
): UseBucketImageResult;
export function useBucketImage<S extends SchemaStructure>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseBucketImageOptions
): UseBucketImageResult;
export function useBucketImage<S extends SchemaStructure>(
  dbOrBucketName: SyncedDb<S> | BucketNames<S>,
  bucketNameOrPath?: BucketNames<S> | Accessor<string | null | undefined>,
  pathOrOptions?: Accessor<string | null | undefined> | UseBucketImageOptions,
  maybeOptions?: UseBucketImageOptions
): UseBucketImageResult {
  let db: SyncedDb<S>;
  let bucketName: BucketNames<S>;
  let path: Accessor<string | null | undefined>;
  let options: UseBucketImageOptions;

  if (typeof dbOrBucketName === 'string') {
    db = useDb<S>();
    bucketName = dbOrBucketName as BucketNames<S>;
    path = bucketNameOrPath as Accessor<string | null | undefined>;
    options = (pathOrOptions as UseBucketImageOptions) ?? {};
  } else {
    db = dbOrBucketName as SyncedDb<S>;
    bucketName = bucketNameOrPath as BucketNames<S>;
    path = pathOrOptions as Accessor<string | null | undefined>;
    options = maybeOptions ?? {};
  }

  // Registered BEFORE the download so the sidecar read enters the serialized
  // remote queue first: the placeholder should never wait behind the bytes it
  // is standing in for.
  const wantHash = options.blurhash !== false;
  const { hash } = useBlurhash(db, bucketName, () => (wantHash ? path() : null));

  const file = useDownloadFile(db, bucketName, path, options);

  const [ready, setReady] = createSignal(false);
  // A new url (path change, refetch) means a new undecoded bitmap.
  createEffect(on(file.url, () => setReady(false), { defer: true }));

  const gate = (img: HTMLImageElement) => {
    const done = () => setReady(true);
    if (typeof img.decode === 'function') {
      img.decode().then(done, done);
    } else if (img.complete) {
      done();
    } else {
      img.onload = done;
      img.onerror = done;
    }
  };

  return { ...file, blurhash: hash, ready, gate };
}
