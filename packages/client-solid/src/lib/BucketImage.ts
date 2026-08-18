import { createSignal, createEffect, onMount, onCleanup } from 'solid-js';
import type { JSX } from 'solid-js';
import type { BucketNames, SchemaStructure } from '@spooky-sync/query-builder';
import { useBucketImage, type UseBucketImageOptions } from './use-bucket-image';
import { Blurhash } from './Blurhash';

export interface BucketImageProps {
  /** Bucket name from the schema. */
  bucket: string;
  /** Path within the bucket. Nullish renders only the fallback layers. */
  path: string | null | undefined;
  alt?: string;
  /** Classes for the container element (sizing/positioning). */
  class?: string;
  /** Classes for the inner `<img>` (the layout styles are inline). */
  imgClass?: string;
  /** `object-fit` for the image. Defaults to `cover`. */
  fit?: 'cover' | 'contain' | 'fill' | 'none' | 'scale-down';
  /**
   * Bottom placeholder layer (your own plate/skeleton), shown until the image
   * settles. The blurhash layer paints on top of it once the sidecar resolves.
   */
  fallback?: JSX.Element;
  /** Crossfade duration in ms. Defaults to 300. */
  transition?: number;
  /** Crossfade easing. Defaults to an ease-out-expo curve. */
  easing?: string;
  /** Resolve the blurhash sidecar. Defaults to true. */
  blurhash?: boolean;
  /** Download tuning, forwarded to the underlying `useDownloadFile`. */
  options?: UseBucketImageOptions;
}

const LAYER_STYLE = 'position:absolute;inset:0;width:100%;height:100%;';

/**
 * A bucket image that never pops in: it layers (bottom to top) your `fallback`
 * plate, the automatically stored blurhash, and the real image, which stays
 * transparent until the bitmap is DECODED and then crossfades over the
 * placeholders. Placeholder layers unmount once the fade settles. Respects
 * prefers-reduced-motion (instant swap). The container is made
 * `position: relative` unless your `class` positions it already.
 *
 * ```tsx
 * <BucketImage bucket="covers" path={row.cover_key} class="absolute inset-0"
 *              fallback={<MyPlate />} alt="" />
 * ```
 */
export function BucketImage(props: BucketImageProps): JSX.Element {
  if (typeof document === 'undefined') return null;

  const image = useBucketImage(
    props.bucket as BucketNames<SchemaStructure>,
    () => props.path,
    { ...props.options, blurhash: props.blurhash !== false }
  );

  const reducedMotion =
    typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

  const root = document.createElement('div');
  createEffect(() => {
    root.className = props.class ?? '';
  });
  // Layers are absolutely positioned; give them an anchor without stomping on
  // a caller class that already positions the container (inline would win).
  onMount(() => {
    if (getComputedStyle(root).position === 'static') root.style.position = 'relative';
  });

  const placeholder = document.createElement('div');
  placeholder.style.cssText = LAYER_STYLE;
  const fallback = props.fallback;
  if (fallback != null) {
    for (const node of Array.isArray(fallback) ? fallback : [fallback]) {
      if (node instanceof Node) placeholder.append(node);
    }
  }
  const hashCanvas = Blurhash({
    get hash() {
      return image.blurhash();
    },
    style: LAYER_STYLE,
  });
  if (hashCanvas instanceof Node) placeholder.append(hashCanvas);

  const img = document.createElement('img');
  img.decoding = 'async';
  img.style.cssText = `${LAYER_STYLE}opacity:0;`;
  createEffect(() => {
    img.className = props.imgClass ?? '';
  });
  createEffect(() => {
    img.style.objectFit = props.fit ?? 'cover';
  });
  createEffect(() => {
    img.alt = props.alt ?? '';
  });
  createEffect(() => {
    img.style.transition = reducedMotion
      ? 'none'
      : `opacity ${props.transition ?? 300}ms ${props.easing ?? 'cubic-bezier(0.16, 1, 0.3, 1)'}`;
  });
  createEffect(() => {
    const url = image.url();
    if (!url) {
      img.removeAttribute('src');
      return;
    }
    img.src = url;
    image.gate(img);
  });
  createEffect(() => {
    img.style.opacity = image.ready() ? '1' : '0';
  });

  // Placeholders leave the DOM once the fade is over (a shelf of covers should
  // not composite three layers each forever) and come back when the path
  // changes mid-life (`ready` re-arms via the hook).
  const [settled, setSettled] = createSignal(false);
  createEffect(() => {
    if (!image.ready()) {
      setSettled(false);
      return;
    }
    const wait = (reducedMotion ? 0 : (props.transition ?? 300)) + 120;
    const timer = setTimeout(() => setSettled(true), wait);
    onCleanup(() => clearTimeout(timer));
  });
  createEffect(() => {
    if (settled()) placeholder.remove();
    else if (!placeholder.isConnected) root.insertBefore(placeholder, img);
  });

  root.append(placeholder, img);
  return root;
}
