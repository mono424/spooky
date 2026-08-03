/**
 * Byte storage for cached bucket files.
 *
 * The default implementation is OPFS. Bucket files never arrive over HTTP in
 * this client — `BucketHandle.get()` is a SurrealQL RPC on the sync socket — so
 * neither the browser's HTTP cache nor the Cache API can hold them. We persist
 * the bytes ourselves, and OPFS is the cheapest place to put them: a read is
 * `getFile()` → a disk-backed lazy `File` that `URL.createObjectURL` can serve
 * without ever moving the bytes through the JS heap.
 *
 * Layout is real nested directories rather than one hashed filename:
 *
 *   sp00ky-blobs/<namespace>/<bucket>/<...path segments>
 *
 * That costs a `getDirectoryHandle` per segment on write, and buys the property
 * the whole orphan story rests on: the full `(bucket, path)` key is recoverable
 * from a directory walk alone. The `_00_blob` manifest can therefore be wiped
 * (memory fallback, SQLite pool wipe, IndexedDB corruption recovery) and be
 * rebuilt from disk instead of taking the cached bytes down with it.
 */

/** Identifies one cached file: the bucket it lives in and its path within. */
export interface BlobKey {
  bucket: string;
  path: string;
}

/** What a directory walk can tell us about a stored file, with no manifest. */
export interface BlobStat {
  key: BlobKey;
  size: number;
  /** File mtime. Seeds `lastAccess` when a manifest row has to be rebuilt. */
  mtime: number;
}

export interface BlobStore {
  /** False for {@link MemoryBlobStore} and for OPFS-less environments: the
   *  cache still dedupes and serves within a tab, but nothing survives reload. */
  readonly persistent: boolean;
  /** Namespace (the local bucketId) all keys are resolved under. */
  readonly namespace: string;

  read(key: BlobKey): Promise<Blob | null>;
  /** Returns the number of bytes written. Throws on quota exhaustion. */
  write(key: BlobKey, bytes: Blob): Promise<number>;
  remove(key: BlobKey): Promise<void>;
  /** Every committed file under the current namespace. Sweeps torn writes. */
  list(): Promise<BlobStat[]>;
  /** Drop the whole namespace (sign-out with `clearOnSignOut`, or a reset). */
  clear(): Promise<void>;
  /** Point at another namespace. Does not touch the bytes of the old one. */
  setNamespace(namespace: string): void;
}

export const BLOB_ROOT_DIR = 'sp00ky-blobs';

/**
 * Marks a half-written file. Committed names can never contain a literal `.`
 * (see {@link encodeSegment}), so this suffix is unambiguous — anything wearing
 * it at walk time is a write that died before commit, and is swept.
 */
const PART_MARKER = '.part-';

/** OPFS names are capped around 255 bytes; stay well clear. */
const MAX_SEGMENT_LENGTH = 200;
/** Walk guards, mirroring `walkOpfs` in the DevTools storage report. */
const MAX_WALK_DEPTH = 12;
const MAX_WALK_ENTRIES = 5000;

/** A path segment we refuse to store — the caller degrades to no persistence. */
export class BlobKeyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'BlobKeyError';
  }
}

/**
 * Percent-encode a path segment, and escape `.` on top of that. Escaping the
 * dot is what makes {@link PART_MARKER} safe: a real file called `x.part-1`
 * would otherwise be swept as a torn write on the next boot.
 */
export function encodeSegment(segment: string): string {
  const encoded = encodeURIComponent(segment).replace(/\./g, '%2E');
  if (encoded.length > MAX_SEGMENT_LENGTH) {
    throw new BlobKeyError(`path segment too long to store (${encoded.length} > ${MAX_SEGMENT_LENGTH})`);
  }
  return encoded;
}

export function decodeSegment(segment: string): string {
  return decodeURIComponent(segment);
}

/** Path segments, `..`/`.`/empty stripped so a crafted path can't escape the root. */
export function pathSegments(path: string): string[] {
  return path.split('/').filter((s) => s.length > 0 && s !== '.' && s !== '..');
}

/** `<bucket>/<...path>` — the directory chain plus filename for a key. */
export function keySegments(key: BlobKey): string[] {
  const parts = pathSegments(key.path);
  if (parts.length === 0) throw new BlobKeyError(`empty file path for bucket "${key.bucket}"`);
  return [key.bucket, ...parts].map(encodeSegment);
}

/** Stable manifest row id. Mirrors the on-disk layout, decoded. */
export function blobKeyId(key: BlobKey): string {
  return `${key.bucket}/${pathSegments(key.path).join('/')}`;
}

/** Whether OPFS is usable for read AND write in this environment. Writes need
 *  `createWritable`, which arrived late in Safari; without it we run
 *  memory-only rather than dragging in a sync-access-handle worker. */
export function opfsWritableSupported(): boolean {
  if (typeof navigator === 'undefined' || !navigator.storage?.getDirectory) return false;
  const proto = (globalThis as { FileSystemFileHandle?: { prototype?: unknown } }).FileSystemFileHandle
    ?.prototype as { createWritable?: unknown } | undefined;
  return typeof proto?.createWritable === 'function';
}

function supportsHandleMove(): boolean {
  const proto = (globalThis as { FileSystemFileHandle?: { prototype?: unknown } }).FileSystemFileHandle
    ?.prototype as { move?: unknown } | undefined;
  return typeof proto?.move === 'function';
}

/** Minimal structural view of the OPFS handles we touch. */
interface DirHandle {
  kind: 'directory';
  name: string;
  getDirectoryHandle(name: string, opts?: { create?: boolean }): Promise<DirHandle>;
  getFileHandle(name: string, opts?: { create?: boolean }): Promise<FileHandle>;
  removeEntry(name: string, opts?: { recursive?: boolean }): Promise<void>;
  entries(): AsyncIterableIterator<[string, DirHandle | FileHandle]>;
}

interface FileHandle {
  kind: 'file';
  name: string;
  getFile(): Promise<File>;
  createWritable(): Promise<{ write(data: Blob): Promise<void>; close(): Promise<void>; abort?(): Promise<void> }>;
  move?(name: string): Promise<void>;
}

function isNotFound(err: unknown): boolean {
  return err instanceof Error && err.name === 'NotFoundError';
}

/** OPFS-backed byte storage. All calls are async — nothing blocks the main thread. */
export class OpfsBlobStore implements BlobStore {
  readonly persistent = true;
  private ns: string;
  private readonly useTempCommit = supportsHandleMove();
  private tempCounter = 0;
  private readonly tempToken =
    typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID().slice(0, 8) : 'tmp';

  constructor(namespace: string) {
    this.ns = namespace;
  }

  get namespace(): string {
    return this.ns;
  }

  setNamespace(namespace: string): void {
    this.ns = namespace;
  }

  private async root(create: boolean): Promise<DirHandle | null> {
    const opfs = (await navigator.storage.getDirectory()) as unknown as DirHandle;
    return this.resolveDir(opfs, [BLOB_ROOT_DIR, encodeSegment(this.ns)], create);
  }

  private async resolveDir(from: DirHandle, segments: string[], create: boolean): Promise<DirHandle | null> {
    let dir = from;
    for (const segment of segments) {
      try {
        dir = await dir.getDirectoryHandle(segment, { create });
      } catch (err) {
        if (!create && isNotFound(err)) return null;
        throw err;
      }
    }
    return dir;
  }

  async read(key: BlobKey): Promise<Blob | null> {
    const segments = keySegments(key);
    const root = await this.root(false);
    if (!root) return null;
    const dir = await this.resolveDir(root, segments.slice(0, -1), false);
    if (!dir) return null;
    try {
      const handle = await dir.getFileHandle(segments[segments.length - 1]!);
      return await handle.getFile();
    } catch (err) {
      if (isNotFound(err)) return null;
      throw err;
    }
  }

  async write(key: BlobKey, bytes: Blob): Promise<number> {
    const segments = keySegments(key);
    const root = await this.root(true);
    if (!root) throw new Error('OPFS root unavailable');
    const dir = (await this.resolveDir(root, segments.slice(0, -1), true))!;
    const name = segments[segments.length - 1]!;

    // With `move()` we write to a temp name and commit by rename, so a crash
    // mid-write leaves a sweepable `.part-` file rather than a truncated real
    // one. Without it (Firefox), write in place: a torn file is caught on read
    // by the manifest size check, which we need anyway for cross-tab races.
    const target = this.useTempCommit ? `${name}${PART_MARKER}${this.tempToken}-${this.tempCounter++}` : name;
    const handle = await dir.getFileHandle(target, { create: true });
    const writable = await handle.createWritable();
    try {
      await writable.write(bytes);
      await writable.close();
    } catch (err) {
      await writable.abort?.().catch(() => {});
      await dir.removeEntry(target).catch(() => {});
      throw err;
    }
    if (this.useTempCommit) await handle.move!(name);
    return bytes.size;
  }

  async remove(key: BlobKey): Promise<void> {
    const segments = keySegments(key);
    const root = await this.root(false);
    if (!root) return;
    const dir = await this.resolveDir(root, segments.slice(0, -1), false);
    if (!dir) return;
    try {
      await dir.removeEntry(segments[segments.length - 1]!);
    } catch (err) {
      if (!isNotFound(err)) throw err;
    }
    // Directories are left behind deliberately: pruning them would race a
    // concurrent write into the same folder for no measurable space saving.
  }

  async list(): Promise<BlobStat[]> {
    const root = await this.root(false);
    if (!root) return [];
    const out: BlobStat[] = [];
    await this.walk(root, [], out, 0);
    return out;
  }

  private async walk(dir: DirHandle, trail: string[], out: BlobStat[], depth: number): Promise<void> {
    if (depth > MAX_WALK_DEPTH || out.length >= MAX_WALK_ENTRIES) return;
    for await (const [name, handle] of dir.entries()) {
      if (out.length >= MAX_WALK_ENTRIES) return;
      if (handle.kind === 'directory') {
        await this.walk(handle as DirHandle, [...trail, name], out, depth + 1);
        continue;
      }
      // Torn write from a process that died between create and commit.
      if (name.includes(PART_MARKER)) {
        await dir.removeEntry(name).catch(() => {});
        continue;
      }
      // A file directly under the namespace root has no bucket segment.
      if (trail.length === 0) continue;
      try {
        const file = await (handle as FileHandle).getFile();
        out.push({
          key: {
            bucket: decodeSegment(trail[0]!),
            path: [...trail.slice(1), name].map(decodeSegment).join('/'),
          },
          size: file.size,
          mtime: file.lastModified,
        });
      } catch {
        // Locked or vanished mid-walk: skip. Reconcile is best-effort.
      }
    }
  }

  async clear(): Promise<void> {
    const opfs = (await navigator.storage.getDirectory()) as unknown as DirHandle;
    const root = await this.resolveDir(opfs, [BLOB_ROOT_DIR], false);
    if (!root) return;
    try {
      await root.removeEntry(encodeSegment(this.ns), { recursive: true });
    } catch (err) {
      if (!isNotFound(err)) throw err;
    }
  }
}

/**
 * In-memory byte store. Used when OPFS is unavailable (the cache still dedupes
 * and serves within the tab, matching the pre-existing behaviour) and as the
 * test double for {@link BlobCache} — `blob-cache.test.ts` runs in node.
 */
export class MemoryBlobStore implements BlobStore {
  readonly persistent = false;
  private ns: string;
  private readonly files = new Map<string, Map<string, { blob: Blob; mtime: number }>>();
  private clock = 0;

  constructor(namespace = 'anon') {
    this.ns = namespace;
  }

  get namespace(): string {
    return this.ns;
  }

  setNamespace(namespace: string): void {
    this.ns = namespace;
  }

  private bucketFiles(): Map<string, { blob: Blob; mtime: number }> {
    let ns = this.files.get(this.ns);
    if (!ns) {
      ns = new Map();
      this.files.set(this.ns, ns);
    }
    return ns;
  }

  async read(key: BlobKey): Promise<Blob | null> {
    keySegments(key);
    return this.bucketFiles().get(blobKeyId(key))?.blob ?? null;
  }

  async write(key: BlobKey, bytes: Blob): Promise<number> {
    keySegments(key);
    this.bucketFiles().set(blobKeyId(key), { blob: bytes, mtime: ++this.clock });
    return bytes.size;
  }

  async remove(key: BlobKey): Promise<void> {
    this.bucketFiles().delete(blobKeyId(key));
  }

  async list(): Promise<BlobStat[]> {
    const out: BlobStat[] = [];
    for (const [id, entry] of this.bucketFiles()) {
      const slash = id.indexOf('/');
      out.push({
        key: { bucket: id.slice(0, slash), path: id.slice(slash + 1) },
        size: entry.blob.size,
        mtime: entry.mtime,
      });
    }
    return out;
  }

  async clear(): Promise<void> {
    this.files.delete(this.ns);
  }
}
