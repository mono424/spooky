import { useRef, useState, useEffect, useCallback } from 'react';

/**
 * Returns a ref to attach to the container and a `progress` value (0–1)
 * that ramps from 0 (element entering viewport bottom) to 1 (element at
 * viewport center). Scroll listener is only active while the element is
 * visible (gated by IntersectionObserver) and throttled via rAF.
 */
export function useScrollReveal() {
  const ref = useRef<HTMLElement | null>(null);
  const [progress, setProgress] = useState(0);
  const visibleRef = useRef(false);
  const rafRef = useRef<number | null>(null);

  const update = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const viewportH = window.innerHeight;

    // 0 when top of element is at viewport bottom, 1 when at viewport center
    const raw = (viewportH - rect.top) / (viewportH / 2);
    setProgress(Math.min(1, Math.max(0, raw)));
  }, []);

  const onScroll = useCallback(() => {
    if (!visibleRef.current) return;
    if (rafRef.current != null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      update();
    });
  }, [update]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const observer = new IntersectionObserver(
      ([entry]) => {
        visibleRef.current = entry.isIntersecting;
        if (entry.isIntersecting) update();
      },
      { rootMargin: '0px 0px 0px 0px', threshold: 0 },
    );

    observer.observe(el);
    window.addEventListener('scroll', onScroll, { passive: true });

    return () => {
      observer.disconnect();
      window.removeEventListener('scroll', onScroll);
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    };
  }, [onScroll, update]);

  return { ref, progress };
}
