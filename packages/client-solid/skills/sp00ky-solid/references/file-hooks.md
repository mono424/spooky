# File Hooks Reference

## useFileUpload

Upload, download, and manage files in a SurrealDB bucket.

### Signatures

```typescript
// Context-based (inside Sp00kyProvider)
useFileUpload<S>(bucketName: BucketNames<S>): FileUploadResult;

// Explicit db
useFileUpload<S>(db: SyncedDb<S>, bucketName: BucketNames<S>): FileUploadResult;
```

### Return Value

```typescript
interface FileUploadResult {
  isUploading: () => boolean;
  error: () => Error | null;
  clearError: () => void;
  upload: (path: string, file: File | Blob) => Promise<void>;
  download: (path: string) => Promise<string | null>; // Returns object URL
  remove: (path: string) => Promise<void>;
  exists: (path: string) => Promise<boolean>;
}
```

### Validation

If the bucket has `maxSize` or `allowedExtensions` configured in the schema, the hook validates files before upload and sets `error()` on failure.

### Example

```tsx
function AvatarUpload() {
  const { upload, isUploading, error, clearError } = useFileUpload('avatars');

  const handleFile = async (e: Event) => {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (file) {
      await upload(`user/${userId()}/avatar.png`, file);
    }
  };

  return (
    <div>
      <input type="file" onChange={handleFile} disabled={isUploading()} />
      <Show when={error()}>
        <p class="error">{error()!.message}</p>
        <button onClick={clearError}>Dismiss</button>
      </Show>
    </div>
  );
}
```

## useDownloadFile

Reactively download a file from a bucket. Re-fetches when the path changes.

### Signatures

```typescript
interface UseDownloadFileOptions {
  cache?: boolean;                    // default true — false disables every layer
  persist?: boolean;                  // default true — keep bytes in OPFS
  pin?: boolean;                      // exempt from pressure eviction
  revalidate?: 'never' | 'head';      // default 'never' (paths are immutable)
}

// Context-based
useDownloadFile<S>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions,
): UseDownloadFileResult;

// Explicit db
useDownloadFile<S>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: UseDownloadFileOptions,
): UseDownloadFileResult;
```

### Return Value

```typescript
interface UseDownloadFileResult {
  url: Accessor<string | null>;     // Object URL for the file
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  refetch: () => void;              // Force re-download, bypassing every layer
}
```

### Caching

Three layers, checked in order:

1. **Object URLs**, refcounted per `bucket:path` and shared between components. Revoked once the last holder releases and the entry ages out of a 32-entry hot window.
2. **OPFS**, under `sp00ky-blobs/<bucketId>/<bucket>/<path>`. Survives reload, works offline, and is namespaced per signed-in user.
3. **The bucket**, over the sync WebSocket.

Nothing expires on a timer — an image whose row is still cached locally has to stay viewable offline. Bytes are dropped only when the app invalidates the path (`bucket.put`/`bucket.delete` do this automatically), when boot reconcile finds no file behind a row, or when the cache exceeds its byte budget, in which case the least-recently-used unpinned entries that nothing is rendering go first.

Configure with `blobCache: { enabled, maxBytes, clearOnSignOut }` on the client. The budget defaults to `min(512 MB, quota × 0.25)`. Inspect live numbers in the DevTools Storage tab under "Bucket file cache".

`persist: false` keeps the in-tab sharing but writes nothing durable. `cache: false` gives each hook instance a private URL fetched fresh and revoked on unmount.

A bucket path is treated as immutable, which is how paths are normally written (`crypto.randomUUID() + ext`). If your app overwrites a path in place from another device, pass `revalidate: 'head'` — it spends a remote `head()` to compare sizes before trusting the cached copy, and keeps the cached copy when that call fails so going offline never blanks an image.

### Pinning and prefetching

```tsx
const bucket = db.bucket('avatars');
await bucket.prefetch(paths);   // warm the cache for offline use
bucket.pin('logo.png');         // never evicted under pressure
bucket.unpin('logo.png');
await bucket.evict('old.png');  // drop locally, leave the remote file alone
```

### Example

```tsx
function Avatar(props: { path: string | null }) {
  const { url, isLoading } = useDownloadFile('avatars', () => props.path);

  return (
    <Show when={!isLoading()} fallback={<Spinner />}>
      <Show when={url()}>
        <img src={url()!} alt="Avatar" />
      </Show>
    </Show>
  );
}
```
