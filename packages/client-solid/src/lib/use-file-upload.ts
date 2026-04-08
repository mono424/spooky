import { createSignal, onCleanup } from 'solid-js';
import type { SchemaStructure, BucketNames } from '@spooky-sync/query-builder';
import { fileToUint8Array } from '@spooky-sync/core';
import type { SyncedDb } from '../index';
import { useDb } from './context';

export interface FileUploadResult {
  isUploading: () => boolean;
  error: () => Error | null;
  clearError: () => void;
  upload: (path: string, file: File | Blob) => Promise<void>;
  download: (path: string) => Promise<string | null>;
  remove: (path: string) => Promise<void>;
  exists: (path: string) => Promise<boolean>;
}

export function useFileUpload<S extends SchemaStructure>(
  bucketName: BucketNames<S>,
): FileUploadResult;
export function useFileUpload<S extends SchemaStructure>(
  db: SyncedDb<S>,
  bucketName: BucketNames<S>,
): FileUploadResult;
export function useFileUpload<S extends SchemaStructure>(
  dbOrBucketName: SyncedDb<S> | BucketNames<S>,
  maybeBucketName?: BucketNames<S>,
): FileUploadResult {
  let db: SyncedDb<S>;
  let bucketName: BucketNames<S>;

  if (typeof dbOrBucketName === 'string') {
    db = useDb<S>();
    bucketName = dbOrBucketName as BucketNames<S>;
  } else {
    db = dbOrBucketName as SyncedDb<S>;
    // oxlint-disable-next-line no-non-null-assertion
    bucketName = maybeBucketName!;
  }

  const [isUploading, setIsUploading] = createSignal(false);
  const [error, setError] = createSignal<Error | null>(null);

  const objectUrls: string[] = [];
  onCleanup(() => {
    for (const url of objectUrls) {
      URL.revokeObjectURL(url);
    }
  });

  const clearError = () => setError(null);

  const validate = (file: File | Blob): void => {
    const config = db.getBucketConfig(bucketName as string);
    if (!config) return;

    if (config.maxSize !== null && config.maxSize !== undefined && file.size > config.maxSize) {
      const maxMB = (config.maxSize / (1024 * 1024)).toFixed(1);
      throw new Error(`File exceeds maximum size of ${maxMB} MB.`);
    }

    if (config.allowedExtensions && config.allowedExtensions.length > 0) {
      const fileName = (file as File).name;
      if (fileName) {
        const ext = fileName.split('.').pop()?.toLowerCase();
        if (!ext || !config.allowedExtensions.includes(ext)) {
          throw new Error(
            `File type not allowed. Accepted: ${config.allowedExtensions.join(', ')}.`
          );
        }
      }
    }
  };

  const upload = async (path: string, file: File | Blob): Promise<void> => {
    setError(null);
    try {
      validate(file);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
      return;
    }

    setIsUploading(true);
    try {
      const bytes = await fileToUint8Array(file);
      await db.bucket(bucketName).put(path, bytes);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setIsUploading(false);
    }
  };

  const download = async (path: string): Promise<string | null> => {
    setError(null);
    try {
      const content = await db.bucket(bucketName).get(path);
      if (!content) return null;
      const objectUrl = URL.createObjectURL(new Blob([content as BlobPart]));
      objectUrls.push(objectUrl);
      return objectUrl;
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
      return null;
    }
  };

  const remove = async (path: string): Promise<void> => {
    setError(null);
    try {
      await db.bucket(bucketName).delete(path);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  const exists = async (path: string): Promise<boolean> => {
    setError(null);
    try {
      return await db.bucket(bucketName).exists(path);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
      return false;
    }
  };

  return {
    isUploading,
    error,
    clearError,
    upload,
    download,
    remove,
    exists,
  };
}
