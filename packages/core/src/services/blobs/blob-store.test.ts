import { describe, it, expect } from 'vitest';
import {
  BlobKeyError,
  MemoryBlobStore,
  blobKeyId,
  decodeSegment,
  encodeSegment,
  keySegments,
  pathSegments,
} from './blob-store';

describe('key encoding', () => {
  it('round-trips segments that are illegal or ambiguous in a filename', () => {
    for (const raw of ['avatar.png', 'a/b', 'hello world', 'ünïcode.jpg', '..', '%2F', "quote'"]) {
      expect(decodeSegment(encodeSegment(raw))).toBe(raw);
    }
  });

  it('never emits a literal dot', () => {
    // The `.part-` torn-write marker is only unambiguous because committed
    // names cannot contain one. If this stops holding, a real file named
    // `x.part-1` gets swept as a half-written file on the next boot.
    for (const raw of ['avatar.png', 'a.part-1', '...', 'x.tar.gz']) {
      expect(encodeSegment(raw)).not.toContain('.');
    }
  });

  it('rejects a segment too long for the filesystem', () => {
    expect(() => encodeSegment('x'.repeat(201))).toThrow(BlobKeyError);
  });

  it('strips traversal and empty segments so a crafted path cannot escape', () => {
    expect(pathSegments('../../etc/passwd')).toEqual(['etc', 'passwd']);
    expect(pathSegments('a//b/./c')).toEqual(['a', 'b', 'c']);
  });

  it('maps a key to a bucket directory plus the path chain', () => {
    expect(keySegments({ bucket: 'files', path: 'a/b/c.png' })).toEqual([
      'files',
      'a',
      'b',
      'c%2Epng',
    ]);
  });

  it('rejects a key with no filename', () => {
    expect(() => keySegments({ bucket: 'files', path: '/' })).toThrow(BlobKeyError);
  });

  it('normalizes the manifest row id', () => {
    expect(blobKeyId({ bucket: 'files', path: '/a//b.png' })).toBe('files/a/b.png');
  });
});

describe('MemoryBlobStore', () => {
  it('round-trips bytes and lists them back under the right key', async () => {
    const store = new MemoryBlobStore('user-1');
    const key = { bucket: 'files', path: 'nested/dir/photo.jpg' };

    await store.write(key, new Blob(['hello']));

    expect(await (await store.read(key))!.text()).toBe('hello');
    expect(await store.list()).toEqual([{ key, size: 5, mtime: expect.any(Number) }]);
  });

  it('isolates namespaces', async () => {
    const store = new MemoryBlobStore('user-1');
    const key = { bucket: 'files', path: 'photo.jpg' };
    await store.write(key, new Blob(['hello']));

    store.setNamespace('user-2');
    expect(await store.read(key)).toBeNull();

    store.setNamespace('user-1');
    expect(await store.read(key)).not.toBeNull();
  });
});
