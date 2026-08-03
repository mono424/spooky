import type { Logger } from '../logger/index';
import type { LocalStore } from '../database/cache-engine';
import type { BlobCacheOptions } from './blob-cache';
import { BlobCache } from './blob-cache';
import { BlobManifest } from './blob-manifest';
import type { BlobStore } from './blob-store';
import { MemoryBlobStore, OpfsBlobStore, opfsWritableSupported } from './blob-store';

export type { BlobKey, BlobStat, BlobStore } from './blob-store';
export { BLOB_ROOT_DIR, BlobKeyError, MemoryBlobStore, OpfsBlobStore, blobKeyId, opfsWritableSupported } from './blob-store';
export type { BlobEntry } from './blob-manifest';
export { BLOB_TABLE, BlobManifest } from './blob-manifest';
export type { BlobCacheOptions, BlobCacheStats, BlobReadOptions, BlobUrlLease } from './blob-cache';
export { BlobCache } from './blob-cache';

/** Ceiling on the default budget, before the quota fraction is applied. */
const MAX_DEFAULT_BUDGET_BYTES = 512 * 1024 * 1024;
/** Share of the origin quota the blob cache may claim by default. The local
 *  database, the SQLite pool and any app storage share the same quota. */
const QUOTA_FRACTION = 0.25;
/** Used until `resolveBlobBudget()` reports back, and when there is no
 *  `navigator.storage.estimate()` to ask. */
export const FALLBACK_BLOB_BUDGET_BYTES = 128 * 1024 * 1024;

/**
 * Budget from the real origin quota when the browser will tell us, otherwise a
 * conservative constant. Async because `estimate()` is; call it once at init
 * and hand the result to {@link BlobCache.setMaxBytes}.
 */
export async function resolveBlobBudget(configured?: number): Promise<number> {
  if (typeof configured === 'number' && configured > 0) return configured;
  try {
    const { quota } = (await navigator.storage.estimate()) ?? {};
    if (typeof quota === 'number' && quota > 0) {
      return Math.min(MAX_DEFAULT_BUDGET_BYTES, Math.floor(quota * QUOTA_FRACTION));
    }
  } catch {
    /* private mode, or no Storage API: fall through */
  }
  return FALLBACK_BLOB_BUDGET_BYTES;
}

export interface CreateBlobCacheOptions {
  local: LocalStore;
  namespace: string;
  logger: Logger;
  fetchRemote: BlobCacheOptions['fetchRemote'];
  headRemote?: BlobCacheOptions['headRemote'];
  maxBytes?: number;
  /** Force a store instead of feature-detecting. Tests and custom engines. */
  store?: BlobStore;
}

/**
 * Build the cache for a client. Falls back to an in-memory store when OPFS
 * cannot be written (Safari before `createWritable`, private modes, non-browser
 * hosts): the cache still dedupes and serves within the tab, which is exactly
 * the behaviour that existed before it — nothing regresses, nothing persists.
 */
export function createBlobCache(opts: CreateBlobCacheOptions): BlobCache {
  const store = opts.store ?? (opfsWritableSupported() ? new OpfsBlobStore(opts.namespace) : new MemoryBlobStore(opts.namespace));
  return new BlobCache({
    store,
    manifest: new BlobManifest(opts.local),
    fetchRemote: opts.fetchRemote,
    headRemote: opts.headRemote,
    logger: opts.logger,
    maxBytes: opts.maxBytes ?? FALLBACK_BLOB_BUDGET_BYTES,
  });
}
