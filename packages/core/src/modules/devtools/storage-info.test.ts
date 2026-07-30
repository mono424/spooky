import { describe, it, expect, afterEach, vi } from 'vitest';
import { walkOpfs } from './storage-info';

/** Minimal in-memory OPFS: directories are nested objects, files are numbers
 *  (their size) or 'locked' (getFile() throws, like a live SAHPool handle). */
type FakeTree = { [name: string]: FakeTree | number | 'locked' };

function makeDirHandle(tree: FakeTree): any {
  return {
    kind: 'directory',
    entries: async function* () {
      for (const [name, node] of Object.entries(tree)) {
        if (typeof node === 'object') {
          yield [name, makeDirHandle(node)];
        } else {
          yield [
            name,
            {
              kind: 'file',
              getFile: async () => {
                if (node === 'locked') throw new DOMException('locked', 'NoModificationAllowedError');
                return { size: node };
              },
            },
          ];
        }
      }
    },
  };
}

function stubOpfs(tree: FakeTree | null) {
  vi.stubGlobal('navigator', tree === null ? {} : {
    storage: { getDirectory: async () => makeDirHandle(tree) },
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('walkOpfs', () => {
  it('reports unsupported without OPFS APIs', async () => {
    stubOpfs(null);
    expect(await walkOpfs()).toEqual({
      supported: false,
      entries: [],
      totalBytes: 0,
      truncated: false,
    });
  });

  it('walks recursively, sums readable sizes, and omits size for locked files', async () => {
    stubOpfs({
      '.sp00ky-anon': { '0000000001': 4096, '0000000002': 'locked' },
      'other.txt': 10,
    });
    const res = await walkOpfs();
    expect(res.supported).toBe(true);
    expect(res.truncated).toBe(false);
    // Locked file present but without a size; total counts only readable bytes.
    expect(res.totalBytes).toBe(4106);
    expect(res.entries).toEqual([
      { path: '.sp00ky-anon', kind: 'directory' },
      { path: '.sp00ky-anon/0000000001', kind: 'file', size: 4096 },
      { path: '.sp00ky-anon/0000000002', kind: 'file' },
      { path: 'other.txt', kind: 'file', size: 10 },
    ]);
  });

  it('caps the listing and flags truncation', async () => {
    const big: FakeTree = {};
    for (let i = 0; i < 10; i++) big[`f${i}`] = 1;
    stubOpfs(big);
    const res = await walkOpfs(5);
    expect(res.truncated).toBe(true);
    expect(res.entries.length).toBe(5);
  });
});
