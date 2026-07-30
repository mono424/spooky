/**
 * Storage diagnostics for the DevTools Storage tab: what engine backs the
 * local cache, whether it actually persists, how much of the device's quota
 * the origin uses, and what is physically sitting in OPFS. Assembled by
 * `DevToolsService.getStorageInfo()`; everything here is JSON-safe.
 */

export interface OpfsEntry {
  /** Path relative to the OPFS root, e.g. `.sp00ky-anon/0000000000000001`. */
  path: string;
  kind: 'file' | 'directory';
  /** Absent when the file's size can't be read (e.g. an exclusive sync access
   *  handle is held on it — exactly the case during SAHPool contention). */
  size?: number;
}

/** Engine-side numbers only the engine can produce (worker round-trips). */
export interface EngineStorageDiagnostics {
  engine: 'sqlite';
  bucketId: string;
  useOpfs: boolean;
  workerSelectConfigured: boolean;
  /** `false` while configured `true` means the runtime downgraded to the
   *  legacy multi-hop select (stale cached worker bundle). */
  workerSelectEffective: boolean;
  /** page_count * page_size. */
  dbSizeBytes?: number;
  /** freelist_count * page_size — reclaimable via VACUUM. */
  freelistBytes?: number;
  tableCounts?: { table: string; rows: number }[];
  error?: string;
}

export interface StorageInfo {
  at: number;
  engine: { kind: 'surrealdb' | 'sqlite' | 'custom'; store: string; bucketId: string };
  health: { status: 'unknown' | 'persistent' | 'memory'; fallback: boolean; error?: string };
  browser: {
    /** `navigator.storage.persisted()` — whether the origin's storage is
     *  exempt from eviction (unrelated to the OPFS pool lock). */
    persisted?: boolean;
    usage?: number;
    quota?: number;
    /** Chrome-only per-system breakdown from `estimate()`. */
    usageDetails?: Record<string, number>;
    error?: string;
  };
  opfs: {
    supported: boolean;
    entries: OpfsEntry[];
    totalBytes: number;
    truncated: boolean;
    error?: string;
  };
  /** Snapshot of `globalThis.__sqliteStats` (SQLite engine only). */
  sqliteStats?: Record<string, unknown>;
  engineDiagnostics?: EngineStorageDiagnostics;
}

/**
 * Recursively list the origin's OPFS. Sizes come from `handle.getFile()`,
 * which throws for a file another context holds an exclusive sync access
 * handle on — SAHPool does exactly that for its whole pool, so a locked file
 * (size omitted) is a live "who has the pool" signal, not a failure.
 */
export async function walkOpfs(maxEntries = 2000, maxDepth = 8): Promise<StorageInfo['opfs']> {
  const nav = typeof navigator !== 'undefined' ? navigator : undefined;
  if (!nav?.storage?.getDirectory) {
    return { supported: false, entries: [], totalBytes: 0, truncated: false };
  }
  const entries: OpfsEntry[] = [];
  let totalBytes = 0;
  let truncated = false;
  try {
    const root = await nav.storage.getDirectory();
    const walk = async (dir: FileSystemDirectoryHandle, prefix: string, depth: number) => {
      if (depth > maxDepth) return;
      // entries() is standard; older lib.dom typings may lack it.
      for await (const [name, handle] of (dir as any).entries() as AsyncIterable<
        [string, FileSystemHandle]
      >) {
        if (entries.length >= maxEntries) {
          truncated = true;
          return;
        }
        const path = prefix ? `${prefix}/${name}` : name;
        if (handle.kind === 'directory') {
          entries.push({ path, kind: 'directory' });
          await walk(handle as FileSystemDirectoryHandle, path, depth + 1);
        } else {
          let size: number | undefined;
          try {
            size = (await (handle as FileSystemFileHandle).getFile()).size;
            totalBytes += size;
          } catch {
            // Locked by an exclusive access handle (e.g. a live SAHPool).
          }
          const entry: OpfsEntry = { path, kind: 'file' };
          if (size !== undefined) entry.size = size;
          entries.push(entry);
        }
      }
    };
    await walk(root, '', 0);
    entries.sort((a, b) => a.path.localeCompare(b.path));
    return { supported: true, entries, totalBytes, truncated };
  } catch (e) {
    return {
      supported: true,
      entries,
      totalBytes,
      truncated,
      error: e instanceof Error ? e.message : String(e),
    };
  }
}
