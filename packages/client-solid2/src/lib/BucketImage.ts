import { createSignal, createEffect, onSettled, children, type Element } from 'solid-js';
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
  fallback?: Element;
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
export function BucketImage(props: BucketImageProps): Element {
  if (typeof document === 'undefined') return null;

  const image = useBucketImage(
    props.bucket as BucketNames<SchemaStructure>,
    () => props.path,
    { ...props.options, blurhash: props.blurhash !== false }
  );

  const reducedMotion =
    typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

  const root = document.createElement('div');
  createEffect(
    () => props.class ?? '',
    (className) => {
      root.className = className;
    }
  );
  // Layers are absolutely positioned; give them an anchor without stomping on
  // a caller class that already positions the container (inline would win).
  onSettled(() => {
    if (getComputedStyle(root).position === 'static') root.style.position = 'relative';
  });

  const placeholder = document.createElement('div');
  placeholder.style.cssText = LAYER_STYLE;
  // Solid hands JSX props over as lazy thunks; `children` resolves them (and
  // nested arrays/functions) to real nodes and keeps them alive reactively.
  const fallbackHolder = document.createElement('div');
  fallbackHolder.style.cssText = LAYER_STYLE;
  placeholder.append(fallbackHolder);
  const resolvedFallback = children(() => props.fallback);
  createEffect(
    () => resolvedFallback.toArray().filter((node) => node instanceof Node) as Node[],
    (nodes) => {
      fallbackHolder.replaceChildren(...nodes);
    }
  );
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
  createEffect(
    () => props.imgClass ?? '',
    (className) => {
      img.className = className;
    }
  );
  createEffect(
    () => props.fit ?? 'cover',
    (fit) => {
      img.style.objectFit = fit;
    }
  );
  createEffect(
    () => props.alt ?? '',
    (alt) => {
      img.alt = alt;
    }
  );
  createEffect(
    () =>
      reducedMotion
        ? 'none'
        : `opacity ${props.transition ?? 300}ms ${props.easing ?? 'cubic-bezier(0.16, 1, 0.3, 1)'}`,
    (transition) => {
      img.style.transition = transition;
    }
  );
  createEffect(
    () => image.url(),
    (url) => {
      if (!url) {
        img.removeAttribute('src');
        return;
      }
      img.src = url;
      image.gate(img);
    }
  );
  createEffect(
    () => image.ready(),
    (ready) => {
      img.style.opacity = ready ? '1' : '0';
    }
  );

  // Placeholders leave the DOM once the fade is over (a shelf of covers should
  // not composite three layers each forever) and come back when the path
  // changes mid-life (`ready` re-arms via the hook). `settled` is written from
  // the apply phase and a timer, outside any tracking scope.
  const [settled, setSettled] = createSignal(false, { ownedWrite: true });
  createEffect(
    () => ({ ready: image.ready(), transition: props.transition ?? 300 }),
    ({ ready, transition }) => {
      if (!ready) {
        setSettled(false);
        return;
      }
      const wait = (reducedMotion ? 0 : transition) + 120;
      const timer = setTimeout(() => setSettled(true), wait);
      return () => clearTimeout(timer);
    }
  );
  createEffect(
    () => settled(),
    (isSettled) => {
      if (isSettled) placeholder.remove();
      else if (!placeholder.isConnected) root.insertBefore(placeholder, img);
    }
  );

  root.append(placeholder, img);
  return root as unknown as Element;
}
