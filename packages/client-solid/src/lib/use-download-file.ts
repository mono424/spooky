import { createSignal, createEffect, onCleanup, type Accessor } from 'solid-js';
import type { SchemaStructure, BucketNames } from '@spooky-sync/query-builder';
import type { SyncedDb } from '../index';
import { useDb } from './context';

export interface UseDownloadFileOptions {
  cache?: boolean;
}

export interface UseDownloadFileResult {
  url: Accessor<string | null>;
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  refetch: () => void;
}

interface CacheEntry {
  url: string;
  refCount: number;
}

const downloadCache = new Map<string, CacheEntry>();
const inflightRequests = new Map<string, Promise<string | null>>();

function cacheKey(bucket: string, path: string): string {
  return `${bucket}:${path}`;
}

function releaseEntry(key: string): void {
  const entry = downloadCache.get(key);
  if (!entry) return;
  entry.refCount--;
  if (entry.refCount <= 0) {
    URL.revokeObjectURL(entry.url);
    downloadCache.delete(key);
  }
}

export function useDownloadFile<S extends SchemaStructure>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions,
): UseDownloadFileResult;
export function useDownloadFile<S extends SchemaStructure>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions,
): UseDownloadFileResult;
export function useDownloadFile<S extends SchemaStructure>(
  dbOrBucketName: SyncedDb<S> | BucketNames<S>,
  bucketNameOrPath?: BucketNames<S> | Accessor<string | null | undefined>,
  pathOrOptions?: Accessor<string | null | undefined> | UseDownloadFileOptions,
  maybeOptions?: UseDownloadFileOptions,
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

  const [url, setUrl] = createSignal<string | null>(null);
  const [isLoading, setIsLoading] = createSignal(false);
  const [error, setError] = createSignal<Error | null>(null);

  let currentKey: string | null = null;
  let privateUrl: string | null = null;
  let refetchTrigger: () => void;
  const [refetchSignal, setRefetchSignal] = createSignal(0);
  refetchTrigger = () => setRefetchSignal((n) => n + 1);

  async function doDownload(key: string, filePath: string): Promise<string | null> {
    if (useCache) {
      // Check cache
      const cached = downloadCache.get(key);
      if (cached) {
        cached.refCount++;
        currentKey = key;
        return cached.url;
      }

      // Check inflight
      const inflight = inflightRequests.get(key);
      if (inflight) {
        const result = await inflight;
        if (result) {
          const entry = downloadCache.get(key);
          if (entry) {
            entry.refCount++;
            currentKey = key;
          }
        }
        return result;
      }

      // Start new download
      const promise = (async () => {
        const content = await db.bucket(bucketName).get(filePath);
        if (!content) return null;
        const objectUrl = URL.createObjectURL(new Blob([content as BlobPart]));
        downloadCache.set(key, { url: objectUrl, refCount: 1 });
        return objectUrl;
      })();

      inflightRequests.set(key, promise);
      try {
        const result = await promise;
        currentKey = key;
        return result;
      } finally {
        inflightRequests.delete(key);
      }
    } else {
      // No caching — private URL per instance
      const content = await db.bucket(bucketName).get(filePath);
      if (!content) return null;
      const objectUrl = URL.createObjectURL(new Blob([content as BlobPart]));
      privateUrl = objectUrl;
      return objectUrl;
    }
  }

  function releaseCurrentEntry() {
    if (useCache && currentKey) {
      releaseEntry(currentKey);
      currentKey = null;
    }
    if (!useCache && privateUrl) {
      URL.revokeObjectURL(privateUrl);
      privateUrl = null;
    }
  }

  createEffect(() => {
    const filePath = path();
    // Subscribe to refetch signal so effect re-runs
    refetchSignal();

    // Release previous entry
    releaseCurrentEntry();

    if (!filePath) {
      setUrl(null);
      setIsLoading(false);
      setError(null);
      return;
    }

    const key = cacheKey(bucketName as string, filePath);

    // Synchronous cache hit
    if (useCache) {
      const cached = downloadCache.get(key);
      if (cached) {
        cached.refCount++;
        currentKey = key;
        setUrl(cached.url);
        setIsLoading(false);
        setError(null);
        return;
      }
    }

    let cancelled = false;
    setIsLoading(true);
    setError(null);

    doDownload(key, filePath).then(
      (result) => {
        if (!cancelled) {
          setUrl(result);
          setIsLoading(false);
        }
      },
      (err) => {
        if (!cancelled) {
          setError(err instanceof Error ? err : new Error(String(err)));
          setIsLoading(false);
        }
      },
    );

    onCleanup(() => {
      cancelled = true;
    });
  });

  onCleanup(() => {
    releaseCurrentEntry();
  });

  const refetch = () => {
    // Evict current entry from cache before re-triggering
    if (useCache && currentKey) {
      const entry = downloadCache.get(currentKey);
      if (entry) {
        URL.revokeObjectURL(entry.url);
        downloadCache.delete(currentKey);
      }
      currentKey = null;
    }
    refetchTrigger();
  };

  return { url, isLoading, error, refetch };
}
