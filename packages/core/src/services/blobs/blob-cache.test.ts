import { describe, it, expect, vi } from 'vitest';
import { BlobCache } from './blob-cache';
import { BlobManifest } from './blob-manifest';
import { MemoryBlobStore, blobKeyId, type BlobKey } from './blob-store';
import { bytes, fakeLocalStore, fakeUrls, silentLogger } from './blob.fixture';

const KEY: BlobKey = { bucket: 'profile_pictures', path: 'avatar.png' };

function setup(opts: { maxBytes?: number; remote?: (key: BlobKey) => Promise<Blob | null> } = {}) {
  const local = fakeLocalStore();
  const store = new MemoryBlobStore('user-1');
  const manifest = new BlobManifest(local.store);
  const urls = fakeUrls();
  let clock = 1000;
  const fetchRemote = vi.fn(opts.remote ?? (async () => bytes(10)));
  const cache = new BlobCache({
    store,
    manifest,
    fetchRemote,
    logger: silentLogger(),
    maxBytes: opts.maxBytes ?? 1_000_000,
    now: () => ++clock,
    urls: urls.factory,
  });
  return { cache, store, manifest, local, urls, fetchRemote };
}

describe('BlobCache reads', () => {
  it('serves the second read from the local store without touching the remote', async () => {
    const { cache, fetchRemote } = setup();

    const first = await cache.read(KEY);
    const second = await cache.read(KEY);

    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(fetchRemote).toHaveBeenCalledTimes(1);
    expect(cache.stats()).toMatchObject({ hits: 1, misses: 1, entries: 1 });
  });

  it('collapses concurrent misses on the same key into one remote read', async () => {
    let release!: (blob: Blob) => void;
    const pending = new Promise<Blob>((resolve) => {
      release = resolve;
    });
    const { cache, fetchRemote } = setup({ remote: () => pending });

    const all = Promise.all([cache.read(KEY), cache.read(KEY), cache.read(KEY)]);
    release(bytes(10));
    const results = await all;

    expect(results.every((r) => r !== null)).toBe(true);
    expect(fetchRemote).toHaveBeenCalledTimes(1);
  });

  it('refetches when the file vanished from the local store behind our back', async () => {
    const { cache, store, fetchRemote } = setup();
    await cache.read(KEY);

    await store.remove(KEY);
    await cache.read(KEY);

    expect(fetchRemote).toHaveBeenCalledTimes(2);
  });

  it('refetches when the stored size disagrees with the manifest', async () => {
    const { cache, manifest, fetchRemote } = setup();
    await cache.read(KEY);

    // What a torn write or a racing tab leaves behind.
    const entry = manifest.getById(blobKeyId(KEY))!;
    manifest.put({ ...entry, size: entry.size + 1 });
    await cache.read(KEY);

    expect(fetchRemote).toHaveBeenCalledTimes(2);
  });

  it('returns null and keeps no row when the file does not exist remotely', async () => {
    const { cache, manifest } = setup({ remote: async () => null });

    expect(await cache.read(KEY)).toBeNull();
    expect(manifest.all()).toHaveLength(0);
  });

  it('reload bypasses the local store', async () => {
    const { cache, fetchRemote } = setup();
    await cache.read(KEY);

    await cache.read(KEY, { reload: true });

    expect(fetchRemote).toHaveBeenCalledTimes(2);
  });

  it('persist:false serves the bytes but writes nothing durable', async () => {
    const { cache, store, manifest } = setup();

    expect(await cache.read(KEY, { persist: false })).not.toBeNull();

    expect(await store.list()).toHaveLength(0);
    expect(manifest.all()).toHaveLength(0);
  });
});

describe('BlobCache revalidation', () => {
  it('drops the cached copy when head() reports a different size', async () => {
    const local = fakeLocalStore();
    const fetchRemote = vi.fn(async () => bytes(10));
    const cache = new BlobCache({
      store: new MemoryBlobStore('user-1'),
      manifest: new BlobManifest(local.store),
      fetchRemote,
      headRemote: async () => ({ size: 999 }),
      logger: silentLogger(),
      maxBytes: 1_000_000,
      urls: fakeUrls().factory,
    });

    await cache.read(KEY);
    await cache.read(KEY, { revalidate: 'head' });

    expect(fetchRemote).toHaveBeenCalledTimes(2);
  });

  it('keeps the cached copy when head() fails — offline must not invalidate', async () => {
    const local = fakeLocalStore();
    const fetchRemote = vi.fn(async () => bytes(10));
    const cache = new BlobCache({
      store: new MemoryBlobStore('user-1'),
      manifest: new BlobManifest(local.store),
      fetchRemote,
      headRemote: async () => {
        throw new Error('offline');
      },
      logger: silentLogger(),
      maxBytes: 1_000_000,
      urls: fakeUrls().factory,
    });

    await cache.read(KEY);
    await cache.read(KEY, { revalidate: 'head' });

    expect(fetchRemote).toHaveBeenCalledTimes(1);
  });
});

describe('BlobCache invalidation', () => {
  it('invalidate removes the bytes, the row and the object URL', async () => {
    const { cache, store, manifest, urls } = setup();
    const lease = await cache.acquireUrl(KEY);
    expect(lease).not.toBeNull();

    await cache.invalidate(KEY);

    expect(await store.list()).toHaveLength(0);
    expect(manifest.all()).toHaveLength(0);
    expect(urls.revoked).toContain(lease!.url);
  });
});

describe('BlobCache eviction', () => {
  const keyN = (n: number): BlobKey => ({ bucket: 'files', path: `f${n}.bin` });

  it('evicts least-recently-used down to the low-water mark, never on age alone', async () => {
    const { cache, manifest } = setup({ maxBytes: 1000, remote: async () => bytes(200) });

    // 5 × 200B = 1000B: at budget, so nothing has been dropped yet.
    for (let i = 0; i < 5; i++) await cache.read(keyN(i));
    expect(manifest.all()).toHaveLength(5);
    expect(cache.stats().evictedEntries).toBe(0);

    // The sixth crosses the budget and triggers a sweep to 80% (800B → 4 files).
    await cache.read(keyN(5));

    expect(manifest.totalBytes()).toBeLessThanOrEqual(800);
    expect(cache.stats().evictedEntries).toBeGreaterThan(0);
    // Oldest goes first; the one just written stays.
    expect(manifest.get(keyN(0))).toBeUndefined();
    expect(manifest.get(keyN(5))).toBeDefined();
  });

  it('never evicts a pinned entry', async () => {
    const { cache, manifest } = setup({ maxBytes: 1000, remote: async () => bytes(200) });

    await cache.read(keyN(0), { pin: true });
    for (let i = 1; i < 6; i++) await cache.read(keyN(i));

    expect(manifest.get(keyN(0))).toBeDefined();
    expect(manifest.get(keyN(0))!.pinned).toBe(true);
  });

  it('never evicts an entry whose object URL is still held', async () => {
    const { cache, manifest } = setup({ maxBytes: 1000, remote: async () => bytes(200) });

    const lease = await cache.acquireUrl(keyN(0));
    for (let i = 1; i < 6; i++) await cache.read(keyN(i));

    expect(manifest.get(keyN(0))).toBeDefined();
    lease!.release();
  });

  it('pauses new writes instead of discarding pinned bytes when over budget', async () => {
    const { cache, manifest } = setup({ maxBytes: 500, remote: async () => bytes(200) });

    for (let i = 0; i < 4; i++) await cache.read(keyN(i), { pin: true });

    expect(cache.stats().persistPaused).toBe(true);
    // Everything that did land is still there — nothing pinned was thrown away.
    expect(manifest.all().every((e) => e.pinned)).toBe(true);
  });
});

describe('BlobCache reconcile', () => {
  it('rebuilds manifest rows from disk when the local store was wiped', async () => {
    const { cache, manifest, local } = setup();
    await cache.read(KEY);
    await cache.flush();
    expect(local.rows.size).toBe(1);

    // The SQLite leader wipe-on-pool-open / memory fallback / IndexedDB
    // recovery case: metadata gone, bytes intact.
    local.rows.clear();
    manifest.reset();
    await cache.reconcile();

    expect(manifest.get(KEY)).toBeDefined();
    expect(cache.stats().reconciledEntries).toBe(1);
  });

  it('serves reads from disk after a reconcile without going remote', async () => {
    const { cache, manifest, local, fetchRemote } = setup();
    await cache.read(KEY);
    local.rows.clear();
    manifest.reset();
    await cache.reconcile();

    await cache.read(KEY);

    expect(fetchRemote).toHaveBeenCalledTimes(1);
  });

  it('survives a local store whose reads throw', async () => {
    const { cache, manifest, local } = setup();
    await cache.read(KEY);
    manifest.reset();
    local.failReads = true;

    await cache.reconcile();

    // Rebuilt purely from disk — a dead manifest must not cost the cached bytes.
    expect(manifest.get(KEY)).toBeDefined();
  });
});

describe('BlobCache object URLs', () => {
  it('shares one URL between holders and revokes it once the hot window ages out', async () => {
    const { cache, urls } = setup({ remote: async () => bytes(10) });

    const a = await cache.acquireUrl(KEY);
    const b = await cache.acquireUrl(KEY);
    expect(a!.url).toBe(b!.url);

    a!.release();
    b!.release();
    // Still warm: a re-mount must not pay for a new URL.
    expect(urls.revoked).not.toContain(a!.url);

    // Push it out of the 32-entry hot window.
    for (let i = 0; i < 33; i++) {
      const lease = await cache.acquireUrl({ bucket: 'files', path: `f${i}.bin` });
      lease!.release();
    }

    expect(urls.revoked).toContain(a!.url);
  });

  it('release is idempotent', async () => {
    const { cache, urls } = setup();
    const lease = await cache.acquireUrl(KEY);

    lease!.release();
    lease!.release();
    lease!.release();

    // A double release must not push the refcount negative and free a URL that
    // another component still holds.
    const again = await cache.acquireUrl(KEY);
    expect(urls.live.has(again!.url)).toBe(true);
  });
});

describe('BlobCache namespaces', () => {
  it('keeps each local bucket separate and leaves the old bucket warm', async () => {
    const { cache, manifest, fetchRemote } = setup();
    await cache.read(KEY);

    await cache.setNamespace('user-2');
    expect(manifest.get(KEY)).toBeUndefined();
    await cache.read(KEY);
    expect(fetchRemote).toHaveBeenCalledTimes(2);

    // Signing back in finds the first user's bytes still there.
    await cache.setNamespace('user-1');
    await cache.read(KEY);
    expect(fetchRemote).toHaveBeenCalledTimes(2);
  });

  it('a read issued before start() finishes waits for the reconcile', async () => {
    // Boot fires start() without awaiting it (the OPFS walk must not delay the
    // WebSocket connect). A read landing mid-walk must still see the rebuilt
    // manifest rather than refetching a file that is already on disk.
    const { cache, manifest, local, fetchRemote } = setup();
    await cache.read(KEY);
    await cache.flush();
    local.rows.clear();
    manifest.reset();

    const starting = cache.start('user-1');
    const readDuringStart = cache.read(KEY);
    await Promise.all([starting, readDuringStart]);

    expect(await readDuringStart).not.toBeNull();
    expect(fetchRemote).toHaveBeenCalledTimes(1);
  });

  it('clear() drops the current namespace', async () => {
    const { cache, store } = setup();
    await cache.read(KEY);

    await cache.clear();

    expect(await store.list()).toHaveLength(0);
  });
});

describe('BlobManifest write-back', () => {
  it('batches access-time updates instead of writing per hit', async () => {
    const { cache, local } = setup();
    await cache.read(KEY);
    await cache.flush();
    const afterFirst = local.upserts;

    for (let i = 0; i < 10; i++) await cache.read(KEY);

    // Nothing flushed yet — the debounce has not fired and no one closed.
    expect(local.upserts).toBe(afterFirst);
    await cache.flush();
    // Ten hits collapse into a single row write.
    expect(local.upserts).toBe(afterFirst + 1);
  });

  it('close() flushes pending metadata', async () => {
    const { cache, local } = setup();
    await cache.read(KEY);

    await cache.close();

    expect(local.rows.get(blobKeyId(KEY))).toMatchObject({ bucket: KEY.bucket, path: KEY.path });
  });
});
