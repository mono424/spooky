import { createEffect, type Element } from 'solid-js';
import { decodeBlurhash } from '@spooky-sync/core';

export interface BlurhashProps {
  /** The blurhash string. Nullish paints nothing (transparent canvas). */
  hash: string | null | undefined;
  /** Decode resolution. 32x32 is plenty: blurhash carries at most 9x9 DCT
   *  components, the canvas is meant to be CSS-scaled to fill. */
  width?: number;
  height?: number;
  /** Contrast punch, see the blurhash reference decoder. Defaults to 1. */
  punch?: number;
  class?: string;
  style?: string;
}

/**
 * A blurhash painted onto a canvas, once per hash change. Size the canvas via
 * `class`/`style` (e.g. `absolute inset-0 w-full h-full`); the internal decode
 * resolution stays tiny regardless of the displayed size.
 */
export function Blurhash(props: BlurhashProps): Element {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');

  createEffect(
    () => props.class ?? '',
    (className) => {
      canvas.className = className;
    }
  );
  createEffect(
    () => props.style ?? '',
    (style) => {
      canvas.style.cssText = style;
    }
  );

  createEffect(
    () => ({
      width: props.width ?? 32,
      height: props.height ?? 32,
      hash: props.hash,
      punch: props.punch ?? 1,
    }),
    ({ width, height, hash, punch }) => {
      canvas.width = width;
      canvas.height = height;
      if (!hash) return;
      try {
        const pixels = decodeBlurhash(hash, width, height, punch);
        const ctx = canvas.getContext('2d');
        if (!ctx) return;
        const imageData = ctx.createImageData(width, height);
        imageData.data.set(pixels);
        ctx.putImageData(imageData, 0, 0);
      } catch {
        // An invalid hash paints nothing; the layer below stays visible.
      }
    }
  );

  return canvas as unknown as Element;
}
