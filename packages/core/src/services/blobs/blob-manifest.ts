import { RecordId } from 'surrealdb';
import type { LocalStore, Row } from '../database/cache-engine';
import type { BlobKey } from './blob-store';
import { blobKeyId } from './blob-store';

/**
 * Metadata index for the blob cache, persisted in the `_00_blob` table of the
 * local engine.
 *
 * The manifest is deliberately NOT the source of truth for existence — OPFS is.
 * Several existing recovery paths destroy the local store while leaving OPFS
 * intact (the SQLite leader's wipe-on-pool-open, the memory fallback when OPFS
 * refuses to open for SQLite, IndexedDB corruption recovery on the SurrealDB
 * engine). Treating the manifest as an index that `reconcile()` can rebuild
 * from a directory walk means those paths cost metadata, not the offline cache.
 *
 * Runtime reads hit the in-memory map; writes are batched. `lastAccess` changes
 * on every single cache hit, so flushing each one would turn an image render
 * into a local-DB write.
 */

export const BLOB_TABLE = '_00_blob';

export interface BlobEntry {
  /** `${bucket}/${path}` — also the `_00_blob` row id. */
  id: string;
  bucket: string;
  path: string;
  size: number;
  contentType: string;
  createdAt: number;
  lastAccess: number;
  hits: number;
  /** Exempt from pressure eviction. Never expires on its own. */
  pinned: boolean;
}

export function entryKey(entry: BlobEntry): BlobKey {
  return { bucket: entry.bucket, path: entry.path };
}

function rowToEntry(id: string, row: Row): BlobEntry | null {
  const bucket = typeof row.bucket === 'string' ? row.bucket : '';
  const path = typeof row.path === 'string' ? row.path : '';
  if (!bucket || !path) return null;
  return {
    id,
    bucket,
    path,
    size: Number(row.size) || 0,
    contentType: typeof row.contentType === 'string' ? row.contentType : '',
    createdAt: Number(row.createdAt) || 0,
    lastAccess: Number(row.lastAccess) || 0,
    hits: Number(row.hits) || 0,
    pinned: row.pinned === true,
  };
}

function entryToRow(entry: BlobEntry): Row {
  return {
    bucket: entry.bucket,
    path: entry.path,
    size: entry.size,
    contentType: entry.contentType,
    createdAt: entry.createdAt,
    lastAccess: entry.lastAccess,
    hits: entry.hits,
    pinned: entry.pinned,
  };
}

export class BlobManifest {
  private entries = new Map<string, BlobEntry>();
  /** Ids whose in-memory state has not been written back yet. */
  private dirty = new Set<string>();
  private removed = new Set<string>();
  private flushing: Promise<void> | null = null;

  constructor(private local: LocalStore) {}

  /**
   * Hydrate from the rows matching `keys`. Ids come from the OPFS listing, so
   * this never needs a full-table scan (and therefore never needs a QueryPlan).
   * Any read failure yields an empty manifest: reconcile then rebuilds every
   * row from disk, which is exactly the desired degradation.
   */
  async load(ids: string[]): Promise<void> {
    this.entries.clear();
    this.dirty.clear();
    this.removed.clear();
    if (ids.length === 0) return;
    let rows: Row[] = [];
    try {
      rows = await this.local.selectByIds(
        BLOB_TABLE,
        ids.map((id) => new RecordId(BLOB_TABLE, id))
      );
    } catch {
      return;
    }
    for (const row of rows) {
      const id = readRowId(row);
      if (!id) continue;
      const entry = rowToEntry(id, row);
      if (entry) this.entries.set(id, entry);
    }
  }

  get(key: BlobKey): BlobEntry | undefined {
    return this.entries.get(blobKeyId(key));
  }

  getById(id: string): BlobEntry | undefined {
    return this.entries.get(id);
  }

  all(): BlobEntry[] {
    return [...this.entries.values()];
  }

  totalBytes(): number {
    let total = 0;
    for (const entry of this.entries.values()) total += entry.size;
    return total;
  }

  pinnedBytes(): number {
    let total = 0;
    for (const entry of this.entries.values()) if (entry.pinned) total += entry.size;
    return total;
  }

  put(entry: BlobEntry): void {
    this.entries.set(entry.id, entry);
    this.removed.delete(entry.id);
    this.dirty.add(entry.id);
  }

  touch(id: string, now: number): void {
    const entry = this.entries.get(id);
    if (!entry) return;
    entry.lastAccess = now;
    entry.hits += 1;
    this.dirty.add(id);
  }

  setPinned(id: string, pinned: boolean): boolean {
    const entry = this.entries.get(id);
    if (!entry || entry.pinned === pinned) return false;
    entry.pinned = pinned;
    this.dirty.add(id);
    return true;
  }

  remove(id: string): void {
    if (!this.entries.delete(id)) return;
    this.dirty.delete(id);
    this.removed.add(id);
  }

  /** Forget everything without scheduling deletes — for a bucket switch, where
   *  the rows belong to the store we are leaving and must stay put. */
  reset(): void {
    this.entries.clear();
    this.dirty.clear();
    this.removed.clear();
  }

  hasPendingWrites(): boolean {
    return this.dirty.size > 0 || this.removed.size > 0;
  }

  /**
   * Write back pending changes. Serialized: a second concurrent flush awaits
   * the first rather than racing it into the same rows. Failures are swallowed
   * on purpose — a lost metadata write costs an LRU timestamp, and the entry is
   * rebuilt from disk on the next reconcile.
   */
  async flush(): Promise<void> {
    if (this.flushing) return this.flushing;
    if (!this.hasPendingWrites()) return;
    const run = this.doFlush();
    this.flushing = run;
    try {
      await run;
    } finally {
      this.flushing = null;
    }
  }

  private async doFlush(): Promise<void> {
    const dirty = [...this.dirty];
    const removed = [...this.removed];
    this.dirty.clear();
    this.removed.clear();
    for (const id of dirty) {
      const entry = this.entries.get(id);
      if (!entry) continue;
      try {
        // A RecordId, never a bare string: the SurrealDB engine binds the id
        // verbatim and `UPSERT <string>` is an InternalError, so a string id
        // silently never lands (see the same note on `_00_preload`).
        await this.local.upsert(BLOB_TABLE, new RecordId(BLOB_TABLE, id), entryToRow(entry), 'replace');
      } catch {
        /* metadata only — rebuilt from disk on the next reconcile */
      }
    }
    for (const id of removed) {
      try {
        await this.local.delete(BLOB_TABLE, new RecordId(BLOB_TABLE, id));
      } catch {
        /* a surviving row with no file is dropped by the next reconcile */
      }
    }
  }
}

/** Row ids come back as a `RecordId` (SurrealDB) or a `table:id` string (SQLite). */
function readRowId(row: Row): string | null {
  const raw = row.id;
  if (raw instanceof RecordId) return String(raw.id);
  if (typeof raw === 'string') {
    const prefix = `${BLOB_TABLE}:`;
    return raw.startsWith(prefix) ? raw.slice(prefix.length) : raw;
  }
  return null;
}
