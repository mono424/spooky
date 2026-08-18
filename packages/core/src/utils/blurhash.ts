import { encode, decode, isBlurhashValid } from 'blurhash';

export { encode as encodeBlurhash, decode as decodeBlurhash, isBlurhashValid };

/**
 * Blurhash generation settings. `true` enables with the defaults below, `false`
 * disables. Resolution order for a put: per-call option > client config >
 * default ON. See {@link Sp00kyConfig.blurhash}.
 */
export type BlurhashSetting = boolean | BlurhashEncodeOptions;

export interface BlurhashEncodeOptions {
  /** Horizontal detail components, 1-9. Defaults to 4. */
  componentX?: number;
  /** Vertical detail components, 1-9. Defaults to 3. */
  componentY?: number;
}

/**
 * Where an image's blurhash lives: a tiny sidecar object in the same bucket.
 * Buckets have no per-object metadata channel (`put` is just `.put($content)`),
 * so the hash for `covers/x_t.webp` is the text object `covers/x_t.webp.bh`.
 */
export function blurhashSidecarPath(path: string): string {
  return `${path}.bh`;
}

/** Extensions `bucket.put` treats as images worth hashing. */
export const BLURHASH_IMAGE_EXTENSIONS = [
  'webp',
  'png',
  'jpg',
  'jpeg',
  'gif',
  'avif',
  'bmp',
] as const;

const IMAGE_EXT_RE = new RegExp(`\\.(${BLURHASH_IMAGE_EXTENSIONS.join('|')})$`, 'i');

export function isImagePath(path: string): boolean {
  return IMAGE_EXT_RE.test(path);
}

/** Longest edge of the downscale the hash is computed from. Blurhash carries at
 *  most 9x9 DCT components, so anything past ~32px is wasted decode work. */
const ENCODE_MAX_EDGE = 32;

/**
 * Decode `content` as an image and compute its blurhash. Browser-only: returns
 * null (never throws) when image decoding is unavailable (node, workers without
 * canvas), when the bytes are not a decodable image, or on any other failure —
 * a missing hash must never break the upload that triggered it.
 */
export async function encodeImageToBlurhash(
  content: string | Uint8Array | Blob,
  options?: BlurhashEncodeOptions
): Promise<string | null> {
  if (typeof createImageBitmap !== 'function') return null;
  let bitmap: ImageBitmap | null = null;
  try {
    const blob =
      content instanceof Blob
        ? content
        : new Blob([content as unknown as BlobPart]);
    bitmap = await createImageBitmap(blob);
    const scale = Math.min(1, ENCODE_MAX_EDGE / Math.max(bitmap.width, bitmap.height));
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const canvas =
      typeof OffscreenCanvas !== 'undefined'
        ? new OffscreenCanvas(width, height)
        : typeof document !== 'undefined'
          ? Object.assign(document.createElement('canvas'), { width, height })
          : null;
    if (!canvas) return null;
    const ctx = canvas.getContext('2d') as
      | OffscreenCanvasRenderingContext2D
      | CanvasRenderingContext2D
      | null;
    if (!ctx) return null;
    ctx.drawImage(bitmap, 0, 0, width, height);
    const { data } = ctx.getImageData(0, 0, width, height);
    return encode(data, width, height, options?.componentX ?? 4, options?.componentY ?? 3);
  } catch {
    return null;
  } finally {
    bitmap?.close();
  }
}
