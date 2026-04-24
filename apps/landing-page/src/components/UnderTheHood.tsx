import { useState, useEffect, useCallback, useRef } from 'react';
import type { RefObject } from 'react';
import { createPortal } from 'react-dom';
import { siRust } from 'simple-icons';
import HologramSticker from 'holographic-sticker';
import { ScrollRevealText } from './ScrollRevealText';

/** Drives the holographic highlight on `.sticker-card` from overall page scroll.
 *  The sweep plays within SCROLL_START..SCROLL_END of the page. Before and after
 *  that window the highlight is held at a muted resting state (EDGE_STRENGTH of
 *  the peak magnitude) so the cards still have some life but feel calmer. */
const SCROLL_START = 0.33;
const SCROLL_END = 0.66;
const EDGE_STRENGTH = 0.36;

const useScrollTilt = (wrapRef: RefObject<HTMLElement | null>) => {
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;

    let card: HTMLElement | null = null;
    let raf = 0;
    const update = () => {
      raf = 0;
      if (!card) card = wrap.querySelector('.sticker-card');
      if (!card) return;
      const scrollEl = document.documentElement;
      const scrollable = scrollEl.scrollHeight - window.innerHeight;
      const pageProgress = scrollable > 0 ? scrollEl.scrollTop / scrollable : 0;
      let t: number;
      if (pageProgress <= SCROLL_START) {
        // Before the window: held at the muted start pose, no motion.
        t = -EDGE_STRENGTH;
      } else if (pageProgress >= SCROLL_END) {
        // After the window: held at the muted end pose, no motion.
        t = EDGE_STRENGTH;
      } else {
        // Inside the window: sweep smoothly from -EDGE to +EDGE so the
        // values match the held states at the 33% / 66% boundaries and
        // there is no visible jump.
        const normalized = (pageProgress - SCROLL_START) / (SCROLL_END - SCROLL_START);
        t = -EDGE_STRENGTH + 2 * EDGE_STRENGTH * normalized;
      }
      card.style.setProperty('--sticker-pointer-x', (-t * 0.9).toFixed(3));
      card.style.setProperty('--sticker-pointer-y', t.toFixed(3));
    };
    const onScroll = () => {
      if (raf) return;
      raf = requestAnimationFrame(update);
    };

    raf = requestAnimationFrame(update);
    window.addEventListener('scroll', onScroll, { passive: true });
    window.addEventListener('resize', onScroll, { passive: true });
    return () => {
      window.removeEventListener('scroll', onScroll);
      window.removeEventListener('resize', onScroll);
      if (raf) cancelAnimationFrame(raf);
    };
  }, [wrapRef]);
};

const svgDataUrl = (inner: string, color: string, viewBox = '0 0 24 24') =>
  `data:image/svg+xml;utf8,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" style="color:${color}">${inner}</svg>`
  )}`;

/** Portrait-card background: retro scene + optional centered icon, shipped as a data URL. */
const cardImageDataUrl = (
  bgInner: string,
  iconInner: string,
  iconColor = '#f8fafc',
  iconY = 300,
  iconScale = 12
) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 700" style="color:${iconColor}">` +
      `<defs><filter id="iconGlow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur in="SourceAlpha" stdDeviation="8"/><feOffset dx="0" dy="0" result="offsetblur"/><feFlood flood-color="#000" flood-opacity="0.45"/><feComposite in2="offsetblur" operator="in"/><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>` +
      bgInner +
      (iconInner
        ? `<g transform="translate(250 ${iconY}) scale(${iconScale}) translate(-12 -12)" filter="url(#iconGlow)">${iconInner}</g>`
        : '') +
    `</svg>`
  )}`;

/** Retro sunset with outrun grid floor. Rust Core. Sun sits at horizon, logo floats in the sky. */
const BG_SUNSET = `
<defs>
  <linearGradient id="r-sky" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1a0436"/>
    <stop offset="0.4" stop-color="#4a0e5e"/>
    <stop offset="0.65" stop-color="#ff3d6b"/>
    <stop offset="0.82" stop-color="#ff8a3d"/>
    <stop offset="1" stop-color="#ffcf6b"/>
  </linearGradient>
  <radialGradient id="r-halo" cx="50%" cy="40%" r="55%">
    <stop offset="0" stop-color="#ffb56b" stop-opacity="0.55"/>
    <stop offset="1" stop-color="#ffb56b" stop-opacity="0"/>
  </radialGradient>
  <linearGradient id="r-sun" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#fff38a"/>
    <stop offset="0.55" stop-color="#ff7a3a"/>
    <stop offset="1" stop-color="#e0245e"/>
  </linearGradient>
  <linearGradient id="r-floor" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#3a0f5a"/>
    <stop offset="1" stop-color="#0f0420"/>
  </linearGradient>
</defs>
<rect width="500" height="700" fill="url(#r-sky)"/>
<circle cx="70" cy="90" r="1.8" fill="#fff" opacity="0.9"/>
<circle cx="420" cy="70" r="1.4" fill="#fff" opacity="0.7"/>
<circle cx="160" cy="140" r="1" fill="#fff" opacity="0.6"/>
<circle cx="390" cy="180" r="1.2" fill="#fff" opacity="0.7"/>
<circle cx="250" cy="260" r="220" fill="url(#r-halo)"/>
<rect x="0" y="490" width="500" height="210" fill="url(#r-floor)"/>
<circle cx="250" cy="540" r="110" fill="url(#r-sun)"/>
<g fill="#1d0a3e">
  <rect x="155" y="500" width="190" height="4"/>
  <rect x="150" y="512" width="200" height="5"/>
  <rect x="145" y="526" width="210" height="6"/>
</g>
<g stroke="#ff5da6" stroke-width="1.6" fill="none" opacity="0.9">
  <line x1="0" y1="490" x2="500" y2="490"/>
  <line x1="0" y1="520" x2="500" y2="520"/>
  <line x1="0" y1="560" x2="500" y2="560"/>
  <line x1="0" y1="612" x2="500" y2="612"/>
  <line x1="0" y1="680" x2="500" y2="680"/>
  <line x1="250" y1="490" x2="-180" y2="700"/>
  <line x1="250" y1="490" x2="40" y2="700"/>
  <line x1="250" y1="490" x2="160" y2="700"/>
  <line x1="250" y1="490" x2="220" y2="700"/>
  <line x1="250" y1="490" x2="280" y2="700"/>
  <line x1="250" y1="490" x2="340" y2="700"/>
  <line x1="250" y1="490" x2="460" y2="700"/>
  <line x1="250" y1="490" x2="680" y2="700"/>
</g>
`.trim();

/** Vaporwave rays + starfield on electric magenta. Instant UI. */
const BG_ELECTRIC = `
<defs>
  <radialGradient id="e-bg" cx="50%" cy="42%" r="75%">
    <stop offset="0" stop-color="#ff66d9"/>
    <stop offset="0.45" stop-color="#8a23c9"/>
    <stop offset="1" stop-color="#1a0246"/>
  </radialGradient>
  <radialGradient id="e-spot" cx="50%" cy="40%" r="35%">
    <stop offset="0" stop-color="#ffe27a" stop-opacity="0.7"/>
    <stop offset="0.5" stop-color="#ff66d9" stop-opacity="0.3"/>
    <stop offset="1" stop-color="#ff66d9" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="500" height="700" fill="url(#e-bg)"/>
<circle cx="250" cy="290" r="220" fill="url(#e-spot)"/>
<g fill="#ffe27a" opacity="0.5">
  <polygon points="250,320 -80,0 160,0"/>
  <polygon points="250,320 210,0 290,0"/>
  <polygon points="250,320 340,0 580,0"/>
</g>
<g fill="#00e5ff" opacity="0.32">
  <polygon points="250,320 -40,120 -40,0 60,0"/>
  <polygon points="250,320 540,120 540,0 440,0"/>
</g>
<g fill="#fff">
  <circle cx="70" cy="100" r="1.8"/>
  <circle cx="420" cy="70" r="2.2"/>
  <circle cx="150" cy="180" r="1.2"/>
  <circle cx="380" cy="210" r="1.5"/>
  <circle cx="90" cy="260" r="1"/>
  <circle cx="450" cy="280" r="1.3"/>
  <circle cx="220" cy="70" r="1"/>
  <circle cx="330" cy="150" r="1.2"/>
</g>
<g stroke="#fff" stroke-width="0.6" opacity="0.08">
  <line x1="0" y1="120" x2="500" y2="120"/>
  <line x1="0" y1="180" x2="500" y2="180"/>
  <line x1="0" y1="240" x2="500" y2="240"/>
  <line x1="0" y1="300" x2="500" y2="300"/>
  <line x1="0" y1="420" x2="500" y2="420"/>
  <line x1="0" y1="480" x2="500" y2="480"/>
  <line x1="0" y1="540" x2="500" y2="540"/>
  <line x1="0" y1="600" x2="500" y2="600"/>
</g>
`.trim();

/** Retro arcade: bright teal gradient, luminous halo, speed streaks, sparkles. Job Scheduler. */
const BG_DIAL = `
<defs>
  <linearGradient id="d-bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#3a7fc4"/>
    <stop offset="0.48" stop-color="#2ba3b8"/>
    <stop offset="1" stop-color="#0a2f42"/>
  </linearGradient>
  <radialGradient id="d-halo" cx="50%" cy="42%" r="48%">
    <stop offset="0" stop-color="#b4fff0" stop-opacity="0.85"/>
    <stop offset="0.5" stop-color="#4df5c9" stop-opacity="0.35"/>
    <stop offset="1" stop-color="#4df5c9" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="500" height="700" fill="url(#d-bg)"/>
<circle cx="250" cy="290" r="230" fill="url(#d-halo)"/>
<g stroke="#4df5c9" opacity="0.55">
  <line x1="0" y1="150" x2="95" y2="150" stroke-width="1.6"/>
  <line x1="405" y1="140" x2="500" y2="140" stroke-width="1.6"/>
  <line x1="0" y1="210" x2="60" y2="210" stroke-width="1.2"/>
  <line x1="440" y1="220" x2="500" y2="220" stroke-width="1.2"/>
  <line x1="0" y1="460" x2="110" y2="460" stroke-width="1.6"/>
  <line x1="390" y1="470" x2="500" y2="470" stroke-width="1.6"/>
  <line x1="0" y1="510" x2="70" y2="510" stroke-width="1.2"/>
  <line x1="430" y1="520" x2="500" y2="520" stroke-width="1.2"/>
</g>
<g fill="#ffd166">
  <path d="M90 110 l3 -9 l3 9 l9 3 l-9 3 l-3 9 l-3 -9 l-9 -3 z"/>
  <path d="M410 130 l2.5 -8 l2.5 8 l8 2.5 l-8 2.5 l-2.5 8 l-2.5 -8 l-8 -2.5 z"/>
  <path d="M140 560 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
  <path d="M380 590 l2.5 -7 l2.5 7 l7 2.5 l-7 2.5 l-2.5 7 l-2.5 -7 l-7 -2.5 z"/>
</g>
<g fill="#fff" opacity="0.6">
  <circle cx="60" cy="80" r="1"/>
  <circle cx="460" cy="90" r="1.2"/>
  <circle cx="190" cy="60" r="0.8"/>
  <circle cx="340" cy="70" r="1"/>
  <circle cx="70" cy="620" r="1"/>
  <circle cx="450" cy="600" r="0.9"/>
  <circle cx="260" cy="600" r="0.8"/>
</g>
<g fill="none" stroke="#ff4fa3" stroke-width="2.5" stroke-linecap="round" opacity="0.9">
  <polyline points="30,50 30,30 50,30"/>
  <polyline points="470,30 450,30 450,50"/>
  <polyline points="30,670 30,680 50,680"/>
  <polyline points="470,680 450,680 450,670"/>
</g>
`.trim();

/** Retro blueprint: indigo grid, purple halo, graph of connected nodes. Typed Everywhere. */
const BG_MEMPHIS = `
<defs>
  <linearGradient id="t-bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1a1b4b"/>
    <stop offset="0.5" stop-color="#0a1540"/>
    <stop offset="1" stop-color="#050924"/>
  </linearGradient>
  <radialGradient id="t-halo" cx="50%" cy="42%" r="55%">
    <stop offset="0" stop-color="#b07bff" stop-opacity="0.55"/>
    <stop offset="0.6" stop-color="#b07bff" stop-opacity="0.15"/>
    <stop offset="1" stop-color="#b07bff" stop-opacity="0"/>
  </radialGradient>
  <pattern id="t-grid" x="0" y="0" width="28" height="28" patternUnits="userSpaceOnUse">
    <path d="M 28 0 L 0 0 0 28" fill="none" stroke="#4b4edb" stroke-width="0.6" opacity="0.35"/>
  </pattern>
</defs>
<rect width="500" height="700" fill="url(#t-bg)"/>
<rect width="500" height="700" fill="url(#t-grid)"/>
<circle cx="250" cy="290" r="240" fill="url(#t-halo)"/>

<g stroke="#ff4bc8" stroke-width="1.4" opacity="0.55" fill="none">
  <line x1="80" y1="120" x2="250" y2="90"/>
  <line x1="250" y1="90" x2="420" y2="130"/>
  <line x1="80" y1="120" x2="100" y2="560"/>
  <line x1="420" y1="130" x2="420" y2="570"/>
  <line x1="100" y1="560" x2="420" y2="570"/>
</g>
<g fill="#ff4bc8">
  <circle cx="80" cy="120" r="4.5"/>
  <circle cx="420" cy="130" r="4.5"/>
  <circle cx="100" cy="560" r="4.5"/>
  <circle cx="420" cy="570" r="4.5"/>
  <circle cx="250" cy="90" r="3.5"/>
</g>
<g fill="#fff" opacity="0.85">
  <circle cx="80" cy="120" r="1.5"/>
  <circle cx="420" cy="130" r="1.5"/>
  <circle cx="100" cy="560" r="1.5"/>
  <circle cx="420" cy="570" r="1.5"/>
</g>

<g fill="#ffd166">
  <path d="M55 320 l2.5 -7 l2.5 7 l7 2.5 l-7 2.5 l-2.5 7 l-2.5 -7 l-7 -2.5 z"/>
  <path d="M440 340 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
  <path d="M200 620 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
</g>

<g stroke="#4df5c9" stroke-width="1.6" opacity="0.85">
  <line x1="20" y1="40" x2="50" y2="40"/>
  <line x1="35" y1="25" x2="35" y2="55"/>
  <line x1="450" y1="40" x2="480" y2="40"/>
  <line x1="465" y1="25" x2="465" y2="55"/>
  <line x1="20" y1="660" x2="50" y2="660"/>
  <line x1="35" y1="645" x2="35" y2="675"/>
  <line x1="450" y1="660" x2="480" y2="660"/>
  <line x1="465" y1="645" x2="465" y2="675"/>
</g>
`.trim();

const Sticker = ({
  title,
  fig,
  iconInner,
  background,
  iconColor,
  iconY,
  iconScale,
  foregroundImage,
  alt,
}: {
  title: string;
  fig: string;
  iconInner: string;
  background: string;
  iconColor?: string;
  iconY?: number;
  iconScale?: number;
  foregroundImage?: string;
  alt: string;
}) => {
  const wrapRef = useRef<HTMLDivElement>(null);
  useScrollTilt(wrapRef);
  return (
  <div ref={wrapRef} className="sp00ky-scroll-wrap">
  <HologramSticker.Root
    className="sp00ky-sticker-root"
    style={{ minHeight: 'auto', ['--sticker-card-width' as string]: '200px' }}
  >
    <HologramSticker.Scene>
      <HologramSticker.Card>
        {/* Layer 1: retro scene with the feature icon baked in */}
        <HologramSticker.ImageLayer
          src={cardImageDataUrl(background, iconInner, iconColor, iconY, iconScale)}
          alt={alt}
          parallax
        />

        {/* Optional foreground image (e.g. a 3D render) */}
        {foregroundImage && (
          <HologramSticker.ImageLayer
            src={foregroundImage}
            alt={alt}
            objectFit="contain"
            parallax
          />
        )}

        {/* Layer 2: holographic shine across the whole card */}
        <HologramSticker.Pattern
          textureUrl="https://assets.codepen.io/605876/figma-texture.png"
          opacity={0.4}
          mixBlendMode="multiply"
        >
          <HologramSticker.Refraction intensity={1} />
        </HologramSticker.Pattern>

        {/* Layer 3: small sp00ky ghost, tiled as the holo watermark */}
        <HologramSticker.Watermark imageUrl="/logo_transparent.svg" opacity={0.2}>
          <HologramSticker.Refraction intensity={1} />
        </HologramSticker.Watermark>

        {/* Layer 4: emboss frame + feature title (top-right) + full sp00ky wordmark (bottom-left) */}
        <HologramSticker.Content>
          <div
            style={{
              position: 'absolute',
              inset: 0,
              zIndex: 2,
              borderRadius: '8cqi',
              opacity: 1,
              filter: 'url(#hologram-lighting)',
              clipPath: 'inset(0 0 0 0 round 8cqi)',
            }}
          >
            {/* Emboss border */}
            <div
              style={{
                position: 'absolute',
                inset: '-1px',
                border: 'calc((8cqi * 0.5) + 1px) solid hsl(0 0% 25%)',
                borderRadius: '8cqi',
                zIndex: 99,
              }}
            />

            {/* Full sp00ky wordmark at bottom-left */}
            <div
              style={{
                position: 'absolute',
                width: 'calc(8cqi * 4)',
                bottom: 'calc(8cqi * 0.85)',
                left: 'calc(8cqi * 0.65)',
                zIndex: 100,
              }}
            >
              <img
                src="/logo.svg"
                alt="sp00ky"
                style={{ width: '100%', display: 'block' }}
              />
            </div>
          </div>
        </HologramSticker.Content>

        {/* Layer 5: Spotlight */}
        <HologramSticker.Spotlight intensity={1} />

        {/* Layer 6: Glare sweep on load */}
        <HologramSticker.Glare />
      </HologramSticker.Card>
    </HologramSticker.Scene>
  </HologramSticker.Root>
  </div>
  );
};

/** SVG filter defs used by the sticker Content layer. Include once per page. */
const StickerFilters = () => (
  <svg className="sr-only" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <defs>
      <filter id="hologram-lighting">
        <feGaussianBlur in="SourceAlpha" stdDeviation="2" result="blur" />
        <feSpecularLighting
          result="lighting"
          in="blur"
          surfaceScale={8}
          specularConstant={12}
          specularExponent={120}
          lightingColor="hsl(0 0% 6%)"
        >
          <fePointLight x={50} y={50} z={300} />
        </feSpecularLighting>
        <feComposite in="lighting" in2="SourceAlpha" operator="in" result="composite" />
        <feComposite
          in="SourceGraphic"
          in2="composite"
          operator="arithmetic"
          k1={0}
          k2={1}
          k3={1}
          k4={0}
          result="litPaint"
        />
      </filter>
      <filter id="hologram-sticker">
        <feMorphology in="SourceAlpha" result="dilate" operator="dilate" radius={2} />
        <feFlood floodColor="hsl(0 0% 100%)" result="outlinecolor" />
        <feComposite in="outlinecolor" in2="dilate" operator="in" result="outlineflat" />
        <feMerge result="merged">
          <feMergeNode in="outlineflat" />
          <feMergeNode in="SourceGraphic" />
        </feMerge>
      </filter>
    </defs>
  </svg>
);

const filled = (d: string) => `<path fill="currentColor" d="${d}"/>`;

const stroked = (body: string, strokeWidth = 1.8) =>
  `<g fill="none" stroke="currentColor" stroke-width="${strokeWidth}" stroke-linecap="round" stroke-linejoin="round">${body}</g>`;

const RUST_SVG = filled(siRust.path);
const BOLT_SVG = filled('M13 2 3 14h9l-1 8 10-12h-9l1-8z');
const CLOCK_SVG = stroked('<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>');
const BRACKETS_SVG = stroked(
  '<polyline points="8 6 2 12 8 18"/><polyline points="16 6 22 12 16 18"/><line x1="14" y1="4" x2="10" y2="20"/>'
);

const features = [
  {
    fig: '0.1',
    title: 'Rust Core',
    subtitle: 'Memory-safe Rust. Zero GC.',
    iconInner: '',
    background: BG_SUNSET,
    iconColor: '#fff6e0',
    foregroundImage: '/rust3d.png',
    alt: 'Rust',
  },
  {
    fig: '0.2',
    title: 'Instant UI',
    subtitle: 'Optimistic writes. WASM speed.',
    iconInner: '',
    background: BG_ELECTRIC,
    iconColor: '#fff36b',
    foregroundImage: '/bolt3d.png',
    alt: 'Instant UI',
  },
  {
    fig: '0.3',
    title: 'Job Scheduler',
    subtitle: 'Durable jobs. Zero wiring.',
    iconInner: '',
    background: BG_DIAL,
    iconColor: '#eafffa',
    foregroundImage: '/clock3d.png',
    alt: 'Job Scheduler',
  },
  {
    fig: '0.4',
    title: 'Typed Everywhere',
    subtitle: 'One schema. Typed end to end.',
    iconInner: '',
    background: BG_MEMPHIS,
    iconColor: '#ecfbff',
    foregroundImage: '/shapes3d.png',
    alt: 'Typed Everywhere',
  },
];

/** 4-column feature grid + "Local first" text — placed after the hero */
export function FeatureGrid() {
  return (
    <>
      <div className="mb-24 md:mb-32">
        <ScrollRevealText
          className="text-4xl md:text-6xl font-semibold leading-tight tracking-tight"
          segments={[
            { text: 'It\'s spooky. ', preRevealed: true },
            { text: 'Data changes, every screen updates. Reactive queries keep every tab, every device, and every user in sync, instantly.' },
          ]}
        />
      </div>

      <StickerFilters />
      <style>{`
        .sp00ky-scroll-wrap { pointer-events: none; }
        .sp00ky-scroll-wrap .sticker-card { animation: none !important; }
        /* Keep the holographic refraction, spotlight, and parallax ghost alive
           even though hover is disabled, so scroll drives the shimmer. */
        .sp00ky-scroll-wrap .sticker-refraction,
        .sp00ky-scroll-wrap .sticker-spotlight:before {
          opacity: 1 !important;
          transition: none !important;
        }
        .sp00ky-scroll-wrap .sticker-img-layer--parallax img {
          translate:
            calc(var(--sticker-pointer-x) * var(--sticker-parallax-img-x, 5%))
            calc(var(--sticker-pointer-y) * var(--sticker-parallax-img-y, 5%)) !important;
          transition: translate 0.35s cubic-bezier(0.2, 0.8, 0.2, 1) !important;
        }
      `}</style>

      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4">
        {features.map((feature, i) => (
          <div
            key={feature.fig}
            className={[
              'px-8 py-6',
              i !== 0 ? 'sm:border-l border-white/[0.15]' : '',
              i !== 0 ? 'border-t sm:border-t-0 border-white/[0.15]' : '',
              i === 2 ? 'sm:border-l-0 md:border-l' : '',
            ].join(' ')}
          >
            <div className="text-[11px] font-mono text-gray-600 uppercase tracking-wider mb-2">
              Fig {feature.fig}
            </div>
            <div className="flex items-center justify-center mb-4">
              <Sticker
                title={feature.title}
                fig={feature.fig}
                iconInner={feature.iconInner}
                background={feature.background}
                iconColor={feature.iconColor}
                iconY={feature.iconY}
                iconScale={feature.iconScale}
                foregroundImage={feature.foregroundImage}
                alt={feature.alt}
              />
            </div>
            <h3 className="text-base font-semibold text-white mb-1">{feature.title}</h3>
            <p className="text-sm text-gray-500">{feature.subtitle}</p>
          </div>
        ))}
      </div>
    </>
  );
}

function DrawerContent() {
  return (
    <div className="p-8 md:p-10 overflow-y-auto h-full">
      <div className="flex justify-between items-baseline mb-6">
        <h3 className="text-lg font-medium tracking-tight text-white">SSP Cluster</h3>
        <span className="text-[10px] font-mono font-medium tracking-wider uppercase text-gray-500 bg-white/[0.03] border border-white/[0.15] px-2.5 py-1 rounded-md">
          Production
        </span>
      </div>

      {/* Cluster Diagram */}
      <div className="w-full flex flex-col gap-4 font-mono mb-8">
        {/* Clients */}
        <div className="flex justify-center gap-8 relative z-10">
          {[
            { label: 'WEB', sub: 'React/Vue' },
            { label: 'APP', sub: 'Flutter/iOS' },
            { label: 'API', sub: 'Backend' },
          ].map((c) => (
            <div key={c.label} className="flex flex-col items-center gap-1">
              <div className="h-8 w-8 border border-white/[0.08] rounded-md bg-white/[0.03] flex items-center justify-center text-gray-500">
                <span className="text-[10px]">{c.label}</span>
              </div>
              <span className="text-[8px] text-gray-600">{c.sub}</span>
            </div>
          ))}
        </div>

        {/* Infrastructure */}
        <div className="border border-dashed border-white/[0.15] p-3 rounded-lg bg-white/[0.02] relative mt-2">
          <div className="absolute -top-2 left-3 bg-[#0a0a0a] px-1.5 text-[8px] text-gray-600 font-medium uppercase tracking-wider">
            Server Infrastructure
          </div>

          <div className="flex flex-col gap-4">
            {/* Top row: SurrealDB — RPC — Scheduler */}
            <div className="flex gap-2 justify-center items-stretch">
              {/* SurrealDB */}
              <div className="border border-white/[0.15] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0">
                <div className="flex items-center justify-between border-b border-white/[0.15] pb-1">
                  <span className="text-[9px] font-bold text-gray-400">SURREALDB</span>
                  <span className="h-1.5 w-1.5 rounded-full bg-gray-500" />
                </div>
                <div className="space-y-1">
                  {['Tables & Auth', 'Live Query Hub', 'Event Triggers'].map((item) => (
                    <div key={item} className="bg-white/[0.03] border border-white/[0.05] px-1.5 py-1 rounded text-[8px] text-gray-400">
                      {item}
                    </div>
                  ))}
                </div>
              </div>

              {/* RPC */}
              <div className="flex flex-col justify-center items-center gap-0.5 text-[8px] text-gray-600/60 w-8 shrink-0">
                <span>RPC</span>
                <div className="w-full h-[1px] bg-white/[0.06]" />
              </div>

              {/* Scheduler */}
              <div className="border border-white/[0.15] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0">
                <div className="flex items-center justify-between border-b border-white/[0.15] pb-1">
                  <span className="text-[9px] font-bold text-gray-400">SCHEDULER</span>
                  <span className="h-1.5 w-1.5 rounded-full bg-gray-500" />
                </div>
                <div className="space-y-1">
                  {['Snapshot Replica', 'WAL', 'Load Balancer', 'Health Monitor', 'Job Scheduler'].map((item) => (
                    <div key={item} className="bg-white/[0.03] border border-white/[0.05] px-1.5 py-0.5 rounded text-[8px] text-gray-400">
                      {item}
                    </div>
                  ))}
                </div>
              </div>
            </div>

            {/* Bottom row: SSP Instances */}
            <div className="flex flex-col gap-1.5">
              {[
                { name: 'SSP-1', status: 'Active' },
                { name: 'SSP-2', status: 'Active' },
                { name: 'SSP-3', status: 'Bootstrapping' },
              ].map((ssp) => (
                <div key={ssp.name} className="border border-white/[0.15] bg-white/[0.02] p-1.5 rounded flex items-center justify-between">
                  <div className="flex items-center gap-1.5">
                    <svg className="w-2.5 h-2.5 flex-shrink-0 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
                      <rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
                    </svg>
                    <span className="text-[9px] font-bold text-gray-400">{ssp.name}</span>
                    <span className="h-1 w-1 rounded-full bg-gray-500" />
                  </div>
                  <span className="text-[8px] text-gray-500">{ssp.status}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>

      {/* Description */}
      <p className="text-[11px] text-gray-500/80 mb-4 font-mono leading-relaxed">
        The Scheduler distributes queries across multiple SSP instances using a persistent
        RocksDB snapshot replica and WAL for crash recovery. Automatic
        load balancing and health monitoring ensure <span className="text-gray-300">zero-downtime deployment</span> and horizontal scalability for enterprise workloads.
      </p>

      {/* Checklist */}
      <ul className="space-y-2 font-mono text-xs text-gray-500 border-t border-white/[0.15] pt-4">
        {[
          'Horizontal Scaling (Add/Remove SSPs).',
          'Zero-Downtime Deployments.',
          'Intelligent Query Routing & Load Balancing.',
        ].map((item) => (
          <li key={item} className="flex items-start gap-2 transition-colors duration-300 hover:text-gray-400">
            <svg className="w-4 h-4 text-gray-600 mt-0.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M5 13l4 4L19 7" />
            </svg>
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** "Horizontally Scalable" text + drawer — placed at end of "How it works" */
export function ScalableText() {
  const [drawerOpen, setDrawerOpen] = useState(false);

  const close = useCallback(() => setDrawerOpen(false), []);

  useEffect(() => {
    if (!drawerOpen) return;

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('keydown', onKey);
    document.body.style.overflow = 'hidden';

    return () => {
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = '';
    };
  }, [drawerOpen, close]);

  return (
    <>
      <div className="mt-16 max-w-3xl">
        <ScrollRevealText
          className="text-2xl md:text-3xl font-semibold leading-snug"
          segments={[
            { text: 'Horizontally Scalable. ', preRevealed: true },
            { text: 'The Scheduler distributes queries across SSP instances with automatic load balancing and zero-downtime deployments.' },
          ]}
          trailing={
            <button
              onClick={() => setDrawerOpen(true)}
              className="text-gray-500 hover:text-gray-300 transition-colors duration-200 inline-flex items-center gap-1"
            >
              Learn more
              <svg className="w-5 h-5 inline" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
                <path strokeLinecap="round" strokeLinejoin="round" d="M13 7l5 5m0 0l-5 5m5-5H6" />
              </svg>
            </button>
          }
        />
      </div>

      {typeof document !== 'undefined' &&
        createPortal(
          <div
            className={`fixed inset-0 z-50 transition-opacity duration-300 ${drawerOpen ? 'opacity-100 pointer-events-auto' : 'opacity-0 pointer-events-none'}`}
          >
            <div
              className="absolute inset-0 bg-black/60 backdrop-blur-sm"
              onClick={close}
            />
            <div
              className={`absolute right-0 top-0 bottom-0 w-full max-w-2xl bg-[#0a0a0a] border-l border-white/[0.15] transition-transform duration-300 ${drawerOpen ? 'translate-x-0' : 'translate-x-full'}`}
            >
              <button
                onClick={close}
                className="absolute top-4 right-4 z-10 text-gray-500 hover:text-gray-300 transition-colors"
              >
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
              <DrawerContent />
            </div>
          </div>,
          document.body
        )}
    </>
  );
}
