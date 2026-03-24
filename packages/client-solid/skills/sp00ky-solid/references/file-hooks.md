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
// Context-based
useDownloadFile<S>(
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: { cache?: boolean },
): UseDownloadFileResult;

// Explicit db
useDownloadFile<S>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
  path: Accessor<string | null | undefined>,
  options?: { cache?: boolean },
): UseDownloadFileResult;
```

### Return Value

```typescript
interface UseDownloadFileResult {
  url: Accessor<string | null>;     // Object URL for the file
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  refetch: () => void;              // Force re-download (evicts cache)
}
```

### Caching

By default, downloads are cached by `bucket:path` key with reference counting. Object URLs are revoked when no component references them. Set `cache: false` to disable.

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
