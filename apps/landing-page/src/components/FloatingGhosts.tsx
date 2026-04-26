import React, { useEffect, useMemo, useRef } from 'react';

const RAW_GHOST = [
  '   ▄▄████████▄▄   ',
  ' ▄██████████████▄ ',
  ' ████████████████ ',
  ' ████  ████  ████ ',
  ' ████████████████ ',
  ' ██████▀  ▀██████ ',
  ' ██████    ██████ ',
  ' ██████▄  ▄██████ ',
  ' ████████████████ ',
  ' ████████████████ ',
  ' ██▄ ▀█▄▀ █▀ ▄█▄█ ',
  '  ▀   █   █   ▀ ▀ ',
];
const EYES_CLOSED = ' ████▀▀████▀▀████ ';

function shiftLine(line: string, dir: number): string {
  const trimmed = line.trim();
  if (dir < -0.7) return trimmed + '  ';
  if (dir > 0.7) return '  ' + trimmed;
  return ' ' + trimmed + ' ';
}

// deterministic PRNG so positions are stable across renders
function mulberry32(seed: number) {
  let a = seed;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface GhostSpec {
  left: number; // %
  top: number; // %
  size: number; // px
  phaseOffset: number;
  blinkOffset: number;
  dx: number; // px drift
  dy: number;
  rot: number; // deg
  duration: number; // s
  delay: number; // s
  opacity: number;
}

function buildGhosts(count: number): GhostSpec[] {
  const rand = mulberry32(424242);
  // jittered grid
  const cols = 5;
  const rows = Math.ceil(count / cols);
  const specs: GhostSpec[] = [];
  for (let i = 0; i < count; i++) {
    const col = i % cols;
    const row = Math.floor(i / cols);
    const cellW = 100 / cols;
    const cellH = 100 / rows;
    const jx = rand();
    const jy = rand();
    specs.push({
      left: col * cellW + cellW * (0.15 + jx * 0.7),
      top: row * cellH + cellH * (0.15 + jy * 0.7),
      size: 2.5 + rand() * 2.5,
      phaseOffset: rand() * 6.28,
      blinkOffset: rand() * 1000,
      dx: (rand() - 0.5) * 60,
      dy: (rand() - 0.5) * 40,
      rot: (rand() - 0.5) * 20,
      duration: 10 + rand() * 14,
      delay: -rand() * 10,
      opacity: 0.35 + rand() * 0.35,
    });
  }
  return specs;
}

const STATIC_ROWS = RAW_GHOST.join('\n');

function MiniGhost({ spec, idx }: { spec: GhostSpec; idx: number }) {
  const ref = useRef<HTMLPreElement>(null);

  // Drive eye-blinks by directly mutating textContent on a slow timer.
  // Avoids React reconciling 14 ghosts on every tick.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (typeof window === 'undefined') return;

    // Only animate blinks if the user hasn't asked for reduced motion.
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduce) return;

    // Stagger blinks per-ghost; ~once every ~6s, closed for ~140ms.
    const period = 5500 + ((spec.blinkOffset * 311) % 4000);
    const offset = (spec.blinkOffset * 173) % period;

    let timer: number | null = null;
    const closedRows = [...RAW_GHOST];
    closedRows[3] = EYES_CLOSED;
    const openText = RAW_GHOST.join('\n');
    const closedText = closedRows.join('\n');

    const tick = () => {
      if (document.hidden) {
        timer = window.setTimeout(tick, 1000);
        return;
      }
      el.textContent = closedText;
      window.setTimeout(() => {
        if (el.isConnected) el.textContent = openText;
      }, 140);
      timer = window.setTimeout(tick, period);
    };
    timer = window.setTimeout(tick, ((offset + idx * 700) % period));

    return () => {
      if (timer != null) clearTimeout(timer);
    };
  }, [spec.blinkOffset, idx]);

  return (
    <pre
      ref={ref}
      className="mini-ghost"
      style={{
        left: `${spec.left}%`,
        top: `${spec.top}%`,
        fontSize: `${spec.size}px`,
        opacity: spec.opacity,
        ['--dx' as any]: `${spec.dx}px`,
        ['--dy' as any]: `${spec.dy}px`,
        ['--rot' as any]: `${spec.rot}deg`,
        animationDuration: `${spec.duration}s`,
        animationDelay: `${spec.delay}s`,
      }}
    >
      {STATIC_ROWS}
    </pre>
  );
}

export function FloatingGhosts({ count = 14 }: { count?: number }) {
  const ghosts = useMemo(() => buildGhosts(count), [count]);
  return (
    <div className="floating-ghosts" aria-hidden="true">
      {ghosts.map((spec, i) => (
        <MiniGhost key={i} spec={spec} idx={i} />
      ))}
    </div>
  );
}

export default FloatingGhosts;
