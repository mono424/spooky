import { describe, it, expect, vi, beforeEach } from 'vitest';
import { BucketHandle } from './sp00ky';
import type { RemoteDatabaseService } from './services/database/index';
import {
  blurhashSidecarPath,
  isImagePath,
  encodeImageToBlurhash,
  encodeBlurhash,
} from './utils/blurhash';

// A real 1x1 hash so isBlurhashValid passes in the read-path tests.
const VALID_HASH = encodeBlurhash(new Uint8ClampedArray([12, 34, 56, 255]), 1, 1, 1, 1);

function makeRemote(queries: string[]) {
  return {
    query: vi.fn(async (surql: string) => {
      queries.push(surql);
      return [null];
    }),
  } as unknown as RemoteDatabaseService;
}

describe('blurhash path helpers', () => {
  it('appends .bh for the sidecar', () => {
    expect(blurhashSidecarPath('a/b_t.webp')).toBe('a/b_t.webp.bh');
  });

  it('recognizes image extensions case-insensitively', () => {
    expect(isImagePath('x/y.webp')).toBe(true);
    expect(isImagePath('x/y.PNG')).toBe(true);
    expect(isImagePath('x/y.webp.bh')).toBe(false);
    expect(isImagePath('x/notes.txt')).toBe(false);
  });
});

describe('encodeImageToBlurhash outside a browser', () => {
  it('returns null instead of throwing (no createImageBitmap in node)', async () => {
    await expect(encodeImageToBlurhash(new Uint8Array([1, 2, 3]))).resolves.toBeNull();
  });
});

describe('BucketHandle.put', () => {
  let queries: string[];

  beforeEach(() => {
    queries = [];
  });

  it('image put succeeds with blurhash null when encoding is unavailable', async () => {
    const handle = new BucketHandle('covers', makeRemote(queries), null);
    const result = await handle.put('a/b_t.webp', new Uint8Array([1]));
    expect(result).toEqual({ blurhash: null });
    // Only the image put went out; no sidecar for a failed/unavailable encode.
    expect(queries).toHaveLength(1);
    expect(queries[0]).toContain('covers:/a/b_t.webp".put');
  });

  it('skips hashing entirely for non-image paths and when disabled', async () => {
    const handle = new BucketHandle('covers', makeRemote(queries), null, { blurhash: false });
    await handle.put('a/b_t.webp', new Uint8Array([1]));
    await handle.put('a/readme.txt', 'hello');
    expect(queries).toHaveLength(2);
    expect(queries.some((q) => q.includes('.bh'))).toBe(false);
  });

  it('writes the sidecar when a hash is produced', async () => {
    // Simulate a browser: a createImageBitmap whose bitmap a canvas can read.
    const pixels = { width: 2, height: 2 };
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn(async () => ({ ...pixels, close: vi.fn() }))
    );
    vi.stubGlobal(
      'OffscreenCanvas',
      class {
        width: number;
        height: number;
        constructor(w: number, h: number) {
          this.width = w;
          this.height = h;
        }
        getContext() {
          return {
            drawImage: () => {},
            getImageData: (_x: number, _y: number, w: number, h: number) => ({
              data: new Uint8ClampedArray(w * h * 4).fill(128),
            }),
          };
        }
      }
    );
    try {
      const handle = new BucketHandle('covers', makeRemote(queries), null);
      const result = await handle.put('a/b_t.webp', new Uint8Array([1]));
      expect(result.blurhash).toBeTruthy();
      expect(queries).toHaveLength(2);
      expect(queries[1]).toContain('covers:/a/b_t.webp.bh".put');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('BucketHandle.blurhash', () => {
  it('returns a valid stored hash and negative-caches misses per path', async () => {
    const reads: string[] = [];
    let payload: unknown = VALID_HASH;
    const remote = {
      query: vi.fn(async (surql: string) => {
        reads.push(surql);
        return [payload];
      }),
    } as unknown as RemoteDatabaseService;
    const handle = new BucketHandle('covers', remote, null);

    await expect(handle.blurhash('hit/x.webp')).resolves.toBe(VALID_HASH);

    payload = null;
    await expect(handle.blurhash('miss/x.webp')).resolves.toBeNull();
    const readsAfterMiss = reads.length;
    // The miss is remembered: no second remote read for the same path.
    await expect(handle.blurhash('miss/x.webp')).resolves.toBeNull();
    expect(reads).toHaveLength(readsAfterMiss);
  });

  it('treats invalid sidecar content as missing', async () => {
    const remote = {
      query: vi.fn(async () => ['not a blurhash \\ at all']),
    } as unknown as RemoteDatabaseService;
    const handle = new BucketHandle('covers', remote, null);
    await expect(handle.blurhash('bad/x.webp')).resolves.toBeNull();
  });
});

describe('BucketHandle.delete', () => {
  it('also deletes the sidecar for image paths, best-effort', async () => {
    const queries: string[] = [];
    const handle = new BucketHandle('covers', makeRemote(queries), null);
    await handle.delete('a/b_t.webp');
    expect(queries).toHaveLength(2);
    expect(queries[0]).toContain('covers:/a/b_t.webp".delete');
    expect(queries[1]).toContain('covers:/a/b_t.webp.bh".delete');

    queries.length = 0;
    await handle.delete('a/notes.txt');
    expect(queries).toHaveLength(1);
  });
});
