import type { Logger } from '../logger/index';
import type { BlobKey, BlobStore } from './blob-store';
import { BlobKeyError, blobKeyId } from './blob-store';
import type { BlobEntry } from './blob-manifest';
import { BlobManifest } from './blob-manifest';

/**
 * Three-layer cache for bucket files.
 *
 *   L0  object URLs, refcounted, per tab      (this class)
 *   L1  bytes in OPFS, durable                (BlobStore)
 *   L2  the remote bucket over the sync WS    (injected `fetchRemote`)
 *
 * Nothing is ever dropped because it got old. A TTL would silently break the
 * offline case this cache exists to serve: the row referencing an image is in
 * the local cache indefinitely, so the image has to be too. Bytes only ever go
 * away for one of four reasons:
 *
 *  1. the app invalidated them — `bucket.put()`/`bucket.delete()` on that path;
 *  2. boot reconcile found no file behind the row (or a torn `.part-` write);
 *  3. a read found a size that disagrees with the manifest;
 *  4. the cache is over budget, and this is the least recently used unpinned
 *     entry that nothing is currently rendering.
 */

/** Evict down to this fraction of the budget, so eviction is not per-write. */
const LOW_WATER = 0.8;
/** Object URLs kept alive at zero references, so list scrolling doesn't thrash. */
const HOT_URL_LIMIT = 32;
/** Parallel remote reads during `prefetch`. Shares the sync socket's queue. */
const PREFETCH_CONCURRENCY = 3;
/** Debounce on manifest write-back. Every cache hit moves `lastAccess`, so
 *  flushing eagerly would turn rendering a list of avatars into a write storm. */
const FLUSH_DEBOUNCE_MS = 2000;

export interface BlobUrlLease {
  url: string;
  release(): void;
}

export interface BlobReadOptions {
  /** Write through to L1 on a miss. Default true. */
  persist?: boolean;
  /** Mark the entry exempt from pressure eviction. */
  pin?: boolean;
  /**
   * Default `'never'`: a bucket path is treated as immutable, which is how the
   * client writes them (`crypto.randomUUID() + ext`). `'head'` spends a remote
   * `head()` to compare sizes before trusting L1.
   */
  revalidate?: 'never' | 'head';
  /** Skip L0/L1 entirely and refill from remote. Backs `refetch()`. */
  reload?: boolean;
}

export interface BlobCacheStats {
  entries: number;
  totalBytes: number;
  budgetBytes: number;
  pinnedBytes: number;
  evictedEntries: number;
  evictedBytes: number;
  reconciledEntries: number;
  hits: number;
  misses: number;
  persistent: boolean;
  /** True when pinned bytes alone exceed the budget: new entries stop being
   *  written rather than pinned ones being thrown away. */
  persistPaused: boolean;
}

export interface BlobCacheOptions {
  store: BlobStore;
  manifest: BlobManifest;
  /** L2 read. Resolves to null when the file does not exist remotely. */
  fetchRemote(key: BlobKey): Promise<Blob | null>;
  /** L2 metadata, for `revalidate: 'head'`. */
  headRemote?(key: BlobKey): Promise<Record<string, unknown> | null>;
  logger: Logger;
  maxBytes: number;
  now?: () => number;
  /** Injected so the URL layer is exercisable off a DOM (node tests). */
  urls?: { create(blob: Blob): string; revoke(url: string): void };
}

interface UrlEntry {
  url: string;
  refCount: number;
}

function defaultUrlFactory(): BlobCacheOptions['urls'] {
  if (typeof URL === 'undefined' || typeof URL.createObjectURL !== 'function') return undefined;
  return {
    create: (blob) => URL.createObjectURL(blob),
    revoke: (url) => URL.revokeObjectURL(url),
  };
}

function isQuotaError(err: unknown): boolean {
  return (
    err instanceof Error &&
    (err.name === 'QuotaExceededError' || err.name === 'NS_ERROR_FILE_NO_DEVICE_SPACE')
  );
}

export class BlobCache {
  private readonly store: BlobStore;
  private readonly manifest: BlobManifest;
  private readonly fetchRemote: (key: BlobKey) => Promise<Blob | null>;
  private readonly headRemote?: (key: BlobKey) => Promise<Record<string, unknown> | null>;
  private readonly logger: Logger;
  private readonly now: () => number;
  private readonly urlFactory: BlobCacheOptions['urls'];

  private maxBytes: number;
  private persistPaused = false;
  /** Set after a quota failure survives one forced eviction. */
  private persistDisabled = false;

  private readonly urls = new Map<string, UrlEntry>();
  /** Ids at zero references, oldest first — the hot-URL window. */
  private idleUrls: string[] = [];
  private readonly inflight = new Map<string, Promise<Blob | null>>();

  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly onPageHide = () => {
    void this.manifest.flush();
  };

  /**
   * Resolves once the manifest has been reconciled against disk. Reads await
   * it, so `start()` does NOT have to be awaited on the boot path — blocking
   * boot on an OPFS directory walk delayed the WebSocket connect (and with it
   * the connection supervisor) for no benefit.
   */
  private ready: Promise<void> = Promise.resolve();

  private hits = 0;
  private misses = 0;
  private evictedEntries = 0;
  private evictedBytes = 0;
  private reconciledEntries = 0;

  constructor(opts: BlobCacheOptions) {
    this.store = opts.store;
    this.manifest = opts.manifest;
    this.fetchRemote = opts.fetchRemote;
    this.headRemote = opts.headRemote;
    this.logger = opts.logger.child({ service: 'BlobCache' });
    this.maxBytes = opts.maxBytes;
    this.now = opts.now ?? (() => Date.now());
    this.urlFactory = opts.urls ?? defaultUrlFactory();
    // A tab that is closed or bfcached never runs the debounce, so the last
    // window of access times would be lost — and `lastAccess` is what eviction
    // orders by. pagehide is the one lifecycle event that reliably fires.
    if (typeof addEventListener === 'function') addEventListener('pagehide', this.onPageHide);
  }

  /** Coalesce manifest write-back. Metadata only, so losing the tail costs an
   *  LRU timestamp that reconcile reseeds from the file mtime. */
  private scheduleFlush(): void {
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      void this.manifest.flush();
    }, FLUSH_DEBOUNCE_MS);
    // Never hold a node process (or a test run) open on a metadata write.
    (this.flushTimer as { unref?: () => void }).unref?.();
  }

  // ---- Reads -------------------------------------------------------------

  /**
   * Resolve the bytes for `key`, filling L1 on the way when `persist` is on.
   * Returns null when the file does not exist remotely and is not cached.
   */
  async read(key: BlobKey, options: BlobReadOptions = {}): Promise<Blob | null> {
    // Boot fires `start()` without awaiting it; a read that lands first must
    // not race the reconcile, or it would refetch a file already on disk and
    // then overwrite the row reconcile is about to rebuild.
    await this.ready;
    const id = blobKeyId(key);
    const persist = options.persist !== false && !this.persistDisabled;

    if (!options.reload) {
      const cached = await this.readLocal(key, id, options);
      if (cached) {
        this.hits++;
        if (options.pin) this.manifest.setPinned(id, true);
        return cached;
      }
    }

    this.misses++;
    const bytes = await this.fetchDeduped(key, id);
    if (!bytes) {
      // Gone remotely (or unreachable with nothing cached): drop any stale row
      // so the next boot doesn't resurrect a file that isn't there.
      await this.dropLocal(key, id);
      return null;
    }
    if (persist) await this.persist(key, id, bytes, options.pin === true);
    return bytes;
  }

  /**
   * An object URL for `key`, refcounted. Callers MUST `release()`; the URL is
   * revoked once the last holder lets go and it falls out of the hot window.
   */
  async acquireUrl(key: BlobKey, options: BlobReadOptions = {}): Promise<BlobUrlLease | null> {
    if (!this.urlFactory) return null;
    const id = blobKeyId(key);

    if (options.reload) this.revokeUrl(id);
    else {
      const live = this.urls.get(id);
      if (live) return this.lease(id, live);
    }

    const blob = await this.read(key, options);
    if (!blob) return null;

    // A concurrent caller may have minted one while we awaited.
    const existing = this.urls.get(id);
    if (existing) return this.lease(id, existing);

    const entry: UrlEntry = { url: this.urlFactory.create(blob), refCount: 0 };
    this.urls.set(id, entry);
    return this.lease(id, entry);
  }

  private lease(id: string, entry: UrlEntry): BlobUrlLease {
    entry.refCount++;
    const idle = this.idleUrls.indexOf(id);
    if (idle !== -1) this.idleUrls.splice(idle, 1);
    let released = false;
    return {
      url: entry.url,
      release: () => {
        if (released) return;
        released = true;
        this.releaseUrl(id);
      },
    };
  }

  private releaseUrl(id: string): void {
    const entry = this.urls.get(id);
    if (!entry) return;
    entry.refCount--;
    if (entry.refCount > 0) return;
    // Keep it warm: re-minting from OPFS is cheap but not free, and a scrolling
    // list unmounts and remounts the same images constantly.
    this.idleUrls.push(id);
    while (this.idleUrls.length > HOT_URL_LIMIT) {
      const evicted = this.idleUrls.shift()!;
      const stale = this.urls.get(evicted);
      if (stale && stale.refCount <= 0) this.revokeUrl(evicted);
    }
  }

  private revokeUrl(id: string): void {
    const entry = this.urls.get(id);
    if (!entry) return;
    this.urls.delete(id);
    const idle = this.idleUrls.indexOf(id);
    if (idle !== -1) this.idleUrls.splice(idle, 1);
    this.urlFactory?.revoke(entry.url);
  }

  /** L1 lookup with the size check that catches torn and cross-tab writes. */
  private async readLocal(key: BlobKey, id: string, options: BlobReadOptions): Promise<Blob | null> {
    let blob: Blob | null;
    try {
      blob = await this.store.read(key);
    } catch (err) {
      if (err instanceof BlobKeyError) return null;
      this.logger.warn({ err, id }, 'blob read from local store failed');
      return null;
    }
    const entry = this.manifest.getById(id);
    if (!blob) {
      if (entry) this.manifest.remove(id);
      return null;
    }
    if (entry && entry.size !== blob.size) {
      // Half-written, or another tab overwrote the path. Disk is not trusted
      // over the manifest here: refill from remote.
      await this.dropLocal(key, id);
      return null;
    }
    if (options.revalidate === 'head' && !(await this.headMatches(key, blob.size))) {
      await this.dropLocal(key, id);
      return null;
    }
    if (entry) {
      this.manifest.touch(id, this.now());
    } else {
      // File on disk with no row — a manifest that was wiped under us. Adopt it
      // rather than re-downloading; reconcile does the same thing at boot.
      this.manifest.put({
        id,
        bucket: key.bucket,
        path: key.path,
        size: blob.size,
        contentType: blob.type ?? '',
        createdAt: this.now(),
        lastAccess: this.now(),
        hits: 1,
        pinned: false,
      });
    }
    this.scheduleFlush();
    return blob;
  }

  /** True when the remote agrees with the cached size, or cannot be reached. */
  private async headMatches(key: BlobKey, size: number): Promise<boolean> {
    if (!this.headRemote) return true;
    try {
      const head = await this.headRemote(key);
      const remoteSize = Number(head?.size);
      return !Number.isFinite(remoteSize) || remoteSize === size;
    } catch {
      // Offline revalidation must not invalidate a good cache entry.
      return true;
    }
  }

  private fetchDeduped(key: BlobKey, id: string): Promise<Blob | null> {
    const running = this.inflight.get(id);
    if (running) return running;
    const promise = this.fetchRemote(key).finally(() => this.inflight.delete(id));
    this.inflight.set(id, promise);
    return promise;
  }

  // ---- Writes ------------------------------------------------------------

  private async persist(key: BlobKey, id: string, bytes: Blob, pin: boolean): Promise<void> {
    // Deliberately NOT gated on `store.persistent`: the memory store still
    // dedupes byte reads across components for the life of the tab, which is
    // the promised degradation. `persistent` is a durability report, not a
    // write permission.
    if (this.persistPaused) return;
    try {
      await this.writeThrough(key, id, bytes, pin);
    } catch (err) {
      if (!isQuotaError(err)) {
        if (!(err instanceof BlobKeyError)) this.logger.warn({ err, id }, 'blob write failed');
        return;
      }
      // Out of space. Make room aggressively, then try exactly once more.
      await this.evictTo(Math.floor(this.maxBytes * 0.5));
      try {
        await this.writeThrough(key, id, bytes, pin);
      } catch (retryErr) {
        this.persistDisabled = true;
        this.logger.warn({ err: retryErr }, 'blob cache disabled: storage quota exhausted');
      }
    }
  }

  private async writeThrough(key: BlobKey, id: string, bytes: Blob, pin: boolean): Promise<void> {
    const size = await this.store.write(key, bytes);
    const now = this.now();
    const previous = this.manifest.getById(id);
    this.manifest.put({
      id,
      bucket: key.bucket,
      path: key.path,
      size,
      contentType: bytes.type ?? '',
      createdAt: previous?.createdAt ?? now,
      lastAccess: now,
      hits: (previous?.hits ?? 0) + 1,
      pinned: pin || previous?.pinned === true,
    });
    this.scheduleFlush();
    await this.enforceBudget();
  }

  // ---- Invalidation and eviction ----------------------------------------

  /** Forget one path everywhere. Called on `bucket.put()`/`bucket.delete()`. */
  async invalidate(key: BlobKey): Promise<void> {
    await this.dropLocal(key, blobKeyId(key));
  }

  private async dropLocal(key: BlobKey, id: string): Promise<void> {
    this.revokeUrl(id);
    this.manifest.remove(id);
    this.scheduleFlush();
    try {
      await this.store.remove(key);
    } catch (err) {
      if (!(err instanceof BlobKeyError)) this.logger.warn({ err, id }, 'blob delete failed');
    }
  }

  setPinned(key: BlobKey, pinned: boolean): void {
    if (this.manifest.setPinned(blobKeyId(key), pinned)) this.scheduleFlush();
  }

  /**
   * Bring total bytes under budget by dropping the least recently used
   * entries. Pinned entries and anything with a live object URL are skipped —
   * evicting bytes that a mounted `<img>` is displaying would blank it.
   */
  private async enforceBudget(): Promise<void> {
    if (this.manifest.totalBytes() <= this.maxBytes) {
      this.persistPaused = false;
      return;
    }
    const remaining = await this.evictTo(Math.floor(this.maxBytes * LOW_WATER));
    if (remaining > this.maxBytes) {
      // Everything left is pinned or on screen. Stop growing rather than
      // throwing away bytes the app explicitly asked us to keep.
      if (!this.persistPaused) {
        this.logger.warn(
          { totalBytes: remaining, budgetBytes: this.maxBytes, pinnedBytes: this.manifest.pinnedBytes() },
          'blob cache over budget with nothing evictable; pausing new writes'
        );
      }
      this.persistPaused = true;
    } else {
      this.persistPaused = false;
    }
  }

  /** Evict LRU-first until at or below `target`. Returns the resulting total. */
  private async evictTo(target: number): Promise<number> {
    let total = this.manifest.totalBytes();
    if (total <= target) return total;
    const candidates = this.manifest
      .all()
      .filter((e) => !e.pinned && (this.urls.get(e.id)?.refCount ?? 0) <= 0)
      .sort((a, b) => a.lastAccess - b.lastAccess);
    const dropped: string[] = [];
    for (const entry of candidates) {
      if (total <= target) break;
      await this.dropLocal({ bucket: entry.bucket, path: entry.path }, entry.id);
      total -= entry.size;
      this.evictedEntries++;
      this.evictedBytes += entry.size;
      dropped.push(entry.id);
    }
    if (dropped.length > 0) {
      this.logger.info(
        { count: dropped.length, totalBytes: total, budgetBytes: this.maxBytes },
        'blob cache evicted least-recently-used entries'
      );
    }
    return total;
  }

  // ---- Lifecycle ---------------------------------------------------------

  /**
   * Rebuild the manifest from what is actually on disk. OPFS wins on existence
   * in both directions: files with no row get a row (seeded from mtime), rows
   * are only loaded for files that exist, and torn `.part-` writes are swept by
   * the walk itself.
   *
   * Rows whose file vanished outside our control (a browser origin eviction)
   * are left in `_00_blob`. They are inert — `load()` only ever asks for ids it
   * found on disk — and are overwritten if that path is cached again.
   */
  async reconcile(): Promise<void> {
    let stats;
    try {
      stats = await this.store.list();
    } catch (err) {
      this.logger.warn({ err }, 'blob cache reconcile failed to list local store');
      return;
    }
    await this.manifest.load(stats.map((s) => blobKeyId(s.key)));
    let rebuilt = 0;
    for (const stat of stats) {
      const id = blobKeyId(stat.key);
      const existing = this.manifest.getById(id);
      if (!existing) {
        this.manifest.put({
          id,
          bucket: stat.key.bucket,
          path: stat.key.path,
          size: stat.size,
          contentType: '',
          createdAt: stat.mtime,
          lastAccess: stat.mtime,
          hits: 0,
          pinned: false,
        });
        rebuilt++;
      } else if (existing.size !== stat.size) {
        this.manifest.put({ ...existing, size: stat.size });
      }
    }
    this.reconciledEntries = stats.length;
    if (rebuilt > 0) {
      this.logger.info({ rebuilt, entries: stats.length }, 'blob cache rebuilt manifest rows from disk');
    }
    await this.enforceBudget();
    await this.manifest.flush();
  }

  /** Warm the cache for offline use. Skips anything already cached. */
  async prefetch(keys: BlobKey[]): Promise<void> {
    const queue = keys.filter((key) => !this.manifest.get(key));
    let cursor = 0;
    const worker = async (): Promise<void> => {
      while (cursor < queue.length) {
        const key = queue[cursor++]!;
        try {
          await this.read(key);
        } catch (err) {
          this.logger.warn({ err, id: blobKeyId(key) }, 'blob prefetch failed');
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(PREFETCH_CONCURRENCY, queue.length) }, worker));
  }

  /** Bind to the boot bucket and hydrate the manifest from disk. Separate from
   *  {@link setNamespace} because boot must reconcile even when the namespace
   *  it lands on is the one the store was constructed with. */
  async start(namespace: string): Promise<void> {
    this.store.setNamespace(namespace);
    // Publish the in-flight reconcile so reads issued before it finishes queue
    // behind it instead of racing it. Swallow here: `reconcile()` already logs,
    // and an unhandled rejection on a fire-and-forget boot call must not
    // surface as a global error.
    this.ready = this.reconcile().catch(() => {});
    await this.ready;
  }

  /** Repoint at another local bucket. The bytes of the old one stay on disk so
   *  switching back (or signing back in) is still warm. */
  async setNamespace(namespace: string): Promise<void> {
    if (namespace === this.store.namespace) return;
    await this.manifest.flush();
    for (const id of [...this.urls.keys()]) this.revokeUrl(id);
    this.inflight.clear();
    this.manifest.reset();
    this.store.setNamespace(namespace);
    this.persistPaused = false;
    this.ready = this.reconcile().catch(() => {});
    await this.ready;
  }

  setMaxBytes(maxBytes: number): void {
    this.maxBytes = maxBytes;
  }

  /** Delete every cached byte in the current namespace. */
  async clear(): Promise<void> {
    for (const id of [...this.urls.keys()]) this.revokeUrl(id);
    for (const entry of this.manifest.all()) this.manifest.remove(entry.id);
    this.inflight.clear();
    try {
      await this.store.clear();
    } catch (err) {
      this.logger.warn({ err }, 'blob cache clear failed');
    }
    await this.manifest.flush();
  }

  async flush(): Promise<void> {
    await this.manifest.flush();
  }

  /** Flush metadata and drop every object URL. Must run before the local store
   *  closes — the flush writes through it. */
  async close(): Promise<void> {
    if (this.flushTimer) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (typeof removeEventListener === 'function') removeEventListener('pagehide', this.onPageHide);
    await this.manifest.flush();
    for (const id of [...this.urls.keys()]) this.revokeUrl(id);
    this.inflight.clear();
  }

  stats(): BlobCacheStats {
    return {
      entries: this.manifest.all().length,
      totalBytes: this.manifest.totalBytes(),
      budgetBytes: this.maxBytes,
      pinnedBytes: this.manifest.pinnedBytes(),
      evictedEntries: this.evictedEntries,
      evictedBytes: this.evictedBytes,
      reconciledEntries: this.reconciledEntries,
      hits: this.hits,
      misses: this.misses,
      persistent: this.store.persistent && !this.persistDisabled,
      persistPaused: this.persistPaused,
    };
  }
}

export type { BlobEntry };
export { BlobManifest };
