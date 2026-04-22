import React, { useEffect, useMemo, useState } from 'react';

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

function MiniGhost({ spec, phase, blinkTick }: { spec: GhostSpec; phase: number; blinkTick: number }) {
  const shouldBlink = ((blinkTick + spec.blinkOffset) | 0) % 23 === 0;
  const rows = RAW_GHOST.map((row, idx) => {
    const wave = Math.sin(idx * 0.6 + phase + spec.phaseOffset);
    const content = shouldBlink && idx === 3 ? EYES_CLOSED : row;
    return shiftLine(content, wave);
  });
  return (
    <pre
      className="mini-ghost"
      style={{
        left: `${spec.left}%`,
        top: `${spec.top}%`,
        fontSize: `${spec.size}px`,
        opacity: spec.opacity,
        // CSS custom properties consumed by .mini-ghost keyframes
        ['--dx' as any]: `${spec.dx}px`,
        ['--dy' as any]: `${spec.dy}px`,
        ['--rot' as any]: `${spec.rot}deg`,
        animationDuration: `${spec.duration}s`,
        animationDelay: `${spec.delay}s`,
      }}
    >
      {rows.join('\n')}
    </pre>
  );
}

export function FloatingGhosts({ count = 14 }: { count?: number }) {
  const ghosts = useMemo(() => buildGhosts(count), [count]);
  const [phase, setPhase] = useState(0);
  const [blinkTick, setBlinkTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setPhase((p) => p + 0.4);
      setBlinkTick((b) => b + 1);
    }, 140);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="floating-ghosts" aria-hidden="true">
      {ghosts.map((spec, i) => (
        <MiniGhost key={i} spec={spec} phase={phase} blinkTick={blinkTick} />
      ))}
    </div>
  );
}

export default FloatingGhosts;
