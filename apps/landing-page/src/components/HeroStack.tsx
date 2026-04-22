import React, { useEffect, useState } from 'react';

const ISO = (x: number, y: number, z = 0): [number, number] => [x - y, (x + y) / 2 - z];

function Sp00kyMark({
  cx = 0,
  cy = 0,
  width = 22,
  fill = 'url(#metalFill)',
  filter = 'url(#engrave)',
}: {
  cx?: number;
  cy?: number;
  width?: number;
  fill?: string;
  filter?: string;
}) {
  const srcW = 51;
  const srcH = 33;
  const scale = width / srcW;
  const height = srcH * scale;
  const tx = cx - width / 2;
  const ty = cy - height / 2;
  return (
    <g transform={`translate(${tx} ${ty}) scale(${scale}) translate(-50 -3)`} filter={filter}>
      <path
        fill={fill}
        d="m62.9 3.5c-6.8 0-12.3 4.9-12.3 17 0 7.8 2.5 15.7 12.3 15.7 7.8 0 11.4-5.1 11.4-15.7 0-8.5-2.2-17-11.4-17zm8.9 18.2c-1.8-0.9-3.7-0.1-4.1 2.4-0.5 3.1-1.1 7-4.9 7-3 0-4.2-2.7-4.7-6.7-0.2-1.9-1.9-4.2-4.8-2.7 0.8-2.2 2.3-3.5 4.4-4.4 0.3-3.2 1.3-8.1 5-8.1 2.8 0 4.6 2.8 5.1 8.1 1.7 0.6 3.3 2.2 4 3.8l0.2 0.5-0.2 0.1z"
      />
      <path
        fill={fill}
        d="m89 3.5c-6.8 0-12.3 4.9-12.3 17 0.4 8.4 2.9 15.6 12.4 15.6 7.8 0 11.9-4.8 11.9-15.6-0.2-8-2.4-17-12-17zm9.5 18.2c-1.8-0.9-4.2-0.3-4.7 2.6-0.4 3.1-1.3 6.7-5 6.8-3.4 0-4.8-2.8-5.3-7-0.3-2-2.2-3.7-4.9-2.4 0.7-1.9 2.3-3.5 4.5-4.4 0.3-3.2 1.5-8 5.8-8 2.8 0 4.8 2.7 5.3 8 1.9 0.7 3.6 2.1 4.4 4.4h-0.1z"
      />
    </g>
  );
}

function TopFace({ cx, cy, children }: { cx: number; cy: number; children: React.ReactNode }) {
  return <g transform={`matrix(1 0.5 -1 0.5 ${cx} ${cy})`}>{children}</g>;
}

function IsoWindow() {
  const w = 140;
  const h = 90;
  const iso = (px: number, py: number) => ISO(px, py, 0);
  const A = iso(-w / 2, -h / 2);
  const B = iso(w / 2, -h / 2);
  const C = iso(w / 2, h / 2);
  const D = iso(-w / 2, h / 2);
  const T = iso(-w / 2, -h / 2 + 18);
  const T2 = iso(w / 2, -h / 2 + 18);

  return (
    <g>
      <polygon
        points={`${A} ${B} ${C} ${D}`}
        fill="#181822"
        stroke="rgba(255,255,255,0.3)"
        strokeWidth="1.1"
        strokeLinejoin="round"
      />
      <polygon
        points={`${A} ${B} ${T2} ${T}`}
        fill="#222232"
        stroke="rgba(255,255,255,0.3)"
        strokeWidth="1.1"
        strokeLinejoin="round"
      />
      {[0, 1, 2].map((i) => {
        const [cx, cy] = iso(-w / 2 + 12 + i * 10, -h / 2 + 9);
        return <circle key={i} cx={cx} cy={cy} r="2" fill="rgba(255,255,255,0.28)" />;
      })}
      {[0, 1, 2, 3].map((i) => {
        const y = -h / 2 + 30 + i * 13;
        const [x1, y1] = iso(-w / 2 + 14, y);
        const widths = [70, 52, 86, 40];
        const [x2, y2] = iso(-w / 2 + 14 + widths[i], y);
        return (
          <line
            key={'ln' + i}
            x1={x1}
            y1={y1}
            x2={x2}
            y2={y2}
            stroke="rgba(255,255,255,0.2)"
            strokeWidth="2"
            strokeLinecap="round"
          />
        );
      })}
      <g>
        {(() => {
          const [x1, y1] = iso(w / 2 - 42, -h / 2 + 32);
          const [x2, y2] = iso(w / 2 - 8, -h / 2 + 32);
          const [x3, y3] = iso(w / 2 - 8, -h / 2 + 58);
          const [x4, y4] = iso(w / 2 - 42, -h / 2 + 58);
          return (
            <polygon
              points={`${x1},${y1} ${x2},${y2} ${x3},${y3} ${x4},${y4}`}
              fill="rgba(155,138,255,0.35)"
              stroke="rgba(155,138,255,0.7)"
              strokeWidth="0.8"
            />
          );
        })()}
        <g transform="matrix(1 0.5 -1 0.5 0 0)">
          <Sp00kyMark cx={w / 2 - 25} cy={-h / 2 + 45} width={22} fill="#ece4ff" filter="" />
        </g>
      </g>
    </g>
  );
}

function IsoMobile() {
  const w = 52;
  const h = 82;
  const iso = (px: number, py: number) => ISO(px, py, 0);
  const A = iso(-w / 2, -h / 2);
  const B = iso(w / 2, -h / 2);
  const C = iso(w / 2, h / 2);
  const D = iso(-w / 2, h / 2);
  return (
    <g>
      <polygon
        points={`${A} ${B} ${C} ${D}`}
        fill="#181822"
        stroke="rgba(255,255,255,0.3)"
        strokeWidth="1.1"
        strokeLinejoin="round"
      />
      {(() => {
        const a = iso(-w / 2 + 4, -h / 2 + 8);
        const b = iso(w / 2 - 4, -h / 2 + 8);
        const c = iso(w / 2 - 4, h / 2 - 8);
        const d = iso(-w / 2 + 4, h / 2 - 8);
        return (
          <polygon
            points={`${a} ${b} ${c} ${d}`}
            fill="#0b0b12"
            stroke="rgba(255,255,255,0.14)"
            strokeWidth="0.6"
          />
        );
      })()}
      {[0, 1, 2, 3, 4].map((i) => {
        const y = -h / 2 + 18 + i * 10;
        const [x1, y1] = iso(-w / 2 + 8, y);
        const widths = [32, 22, 36, 18, 28];
        const [x2, y2] = iso(-w / 2 + 8 + widths[i], y);
        return (
          <line
            key={'m' + i}
            x1={x1}
            y1={y1}
            x2={x2}
            y2={y2}
            stroke="rgba(255,255,255,0.22)"
            strokeWidth="1.6"
            strokeLinecap="round"
          />
        );
      })}
    </g>
  );
}

function IsoSSP({ x = 0, y = 0, label }: { x?: number; y?: number; label?: string }) {
  const w = 36;
  const d = 24;
  const h = 9;
  const top = [
    ISO(x - w / 2, y - d / 2, h),
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
  ];
  const frontRight = [
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x + w / 2, y + d / 2, 0),
    ISO(x + w / 2, y - d / 2, 0),
  ];
  const frontLeft = [
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, 0),
    ISO(x + w / 2, y + d / 2, 0),
  ];
  const [cxTop, cyTop] = ISO(x, y, h);
  return (
    <g filter="url(#softshadow)">
      <ellipse cx={ISO(x, y, 0)[0]} cy={ISO(x, y, 0)[1] + 3} rx="36" ry="16" fill="url(#socGlow)" opacity="0.4" />
      <polygon points={frontRight.join(' ')} fill="url(#socSideR)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.5" strokeLinejoin="round" />
      <polygon points={frontLeft.join(' ')} fill="url(#socSideL)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.5" strokeLinejoin="round" />
      <polygon points={top.join(' ')} fill="url(#socTop)" stroke="rgba(180,160,255,0.22)" strokeWidth="0.5" strokeLinejoin="round" />
      <TopFace cx={cxTop} cy={cyTop}>
        <text
          x={0}
          y={3}
          textAnchor="middle"
          fill="url(#metalFill)"
          filter="url(#engrave)"
          fontFamily="'IBM Plex Sans', system-ui, sans-serif"
          fontSize="8.5"
          fontWeight="700"
          letterSpacing="0.04em"
        >
          {label || 'SSP'}
        </text>
      </TopFace>
    </g>
  );
}

function IsoSnapshot({ x = 0, y = 0 }: { x?: number; y?: number; label?: string }) {
  const w = 42;
  const d = 26;
  const h = 10;
  const top = [
    ISO(x - w / 2, y - d / 2, h),
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
  ];
  const frontRight = [
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x + w / 2, y + d / 2, 0),
    ISO(x + w / 2, y - d / 2, 0),
  ];
  const frontLeft = [
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, 0),
    ISO(x + w / 2, y + d / 2, 0),
  ];
  const [cxTop, cyTop] = ISO(x, y, h);
  return (
    <g filter="url(#softshadow)">
      <ellipse cx={ISO(x, y, 0)[0]} cy={ISO(x, y, 0)[1] + 4} rx="44" ry="20" fill="url(#socGlow)" opacity="0.55" />
      <polygon points={frontRight.join(' ')} fill="url(#socSideR)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.6" strokeLinejoin="round" />
      <polygon points={frontLeft.join(' ')} fill="url(#socSideL)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.6" strokeLinejoin="round" />
      <polygon points={top.join(' ')} fill="url(#socTop)" stroke="rgba(180,160,255,0.25)" strokeWidth="0.6" strokeLinejoin="round" />
      <TopFace cx={cxTop} cy={cyTop}>
        <text
          x={0}
          y={3}
          textAnchor="middle"
          fill="url(#metalFill)"
          filter="url(#engrave)"
          fontFamily="'IBM Plex Sans', system-ui, sans-serif"
          fontSize="9"
          fontWeight="700"
          letterSpacing="0.12em"
        >
          SNAP
        </text>
      </TopFace>
    </g>
  );
}

function IsoScheduler({ x = 0, y = 0 }: { x?: number; y?: number; label?: string }) {
  const w = 56;
  const d = 56;
  const h = 10;
  const top = [
    ISO(x - w / 2, y - d / 2, h),
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
  ];
  const frontRight = [
    ISO(x + w / 2, y - d / 2, h),
    ISO(x + w / 2, y + d / 2, h),
    ISO(x + w / 2, y + d / 2, 0),
    ISO(x + w / 2, y - d / 2, 0),
  ];
  const frontLeft = [
    ISO(x + w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, h),
    ISO(x - w / 2, y + d / 2, 0),
    ISO(x + w / 2, y + d / 2, 0),
  ];
  const [cxTop, cyTop] = ISO(x, y, h);
  return (
    <g filter="url(#softshadow)">
      <ellipse cx={ISO(x, y, 0)[0]} cy={ISO(x, y, 0)[1] + 6} rx="72" ry="34" fill="url(#socGlow)" />
      <ellipse
        cx={ISO(x, y, 0)[0] - 14}
        cy={ISO(x, y, 0)[1] + 10}
        rx="40"
        ry="20"
        fill="url(#socGlowPink)"
        opacity="0.7"
      />
      <polygon points={frontRight.join(' ')} fill="url(#socSideR)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.6" strokeLinejoin="round" />
      <polygon points={frontLeft.join(' ')} fill="url(#socSideL)" stroke="rgba(0,0,0,0.9)" strokeWidth="0.6" strokeLinejoin="round" />
      <polygon points={top.join(' ')} fill="url(#socTop)" stroke="rgba(180,160,255,0.25)" strokeWidth="0.6" strokeLinejoin="round" />
      {(() => {
        const inset = 4;
        const bez = [
          ISO(x - w / 2 + inset, y - d / 2 + inset, h + 0.05),
          ISO(x + w / 2 - inset, y - d / 2 + inset, h + 0.05),
          ISO(x + w / 2 - inset, y + d / 2 - inset, h + 0.05),
          ISO(x - w / 2 + inset, y + d / 2 - inset, h + 0.05),
        ];
        return (
          <polygon
            points={bez.join(' ')}
            fill="none"
            stroke="rgba(200,188,255,0.18)"
            strokeWidth="0.5"
            strokeLinejoin="round"
          />
        );
      })()}
      <TopFace cx={cxTop} cy={cyTop}>
        <Sp00kyMark cx={0} cy={-4} width={28} />
        <text
          x={0}
          y={14}
          textAnchor="middle"
          fill="url(#metalFill)"
          filter="url(#engrave)"
          fontFamily="'IBM Plex Sans', system-ui, sans-serif"
          fontSize="5"
          fontWeight="700"
          letterSpacing="0.3em"
        >
          SCHEDULER
        </text>
      </TopFace>
    </g>
  );
}

function SpookyEngine() {
  const scheduler = { x: 0, y: 0 };
  const snapshot = { x: -80, y: 38 };
  const ssp1 = { x: 78, y: -38 };
  const ssp2 = { x: 78, y: 14 };
  const ssp3 = { x: 78, y: 66 };

  const zTrace = 11;
  const junctionX = scheduler.x + (ssp1.x - scheduler.x) * 0.35;
  const spineY1 = Math.min(ssp1.y, ssp2.y, ssp3.y);
  const spineY2 = Math.max(ssp1.y, ssp2.y, ssp3.y);

  const sp1 = ISO(scheduler.x, scheduler.y, zTrace);
  const spC = ISO(snapshot.x, scheduler.y, zTrace);
  const sp2 = ISO(snapshot.x, snapshot.y, zTrace);
  const snapTrace = `${sp1[0]},${sp1[1]} ${spC[0]},${spC[1]} ${sp2[0]},${sp2[1]}`;

  const t1 = ISO(scheduler.x, scheduler.y, zTrace);
  const t2 = ISO(junctionX, scheduler.y, zTrace);
  const trunk = `${t1[0]},${t1[1]} ${t2[0]},${t2[1]}`;

  const s1 = ISO(junctionX, spineY1, zTrace);
  const s2 = ISO(junctionX, spineY2, zTrace);
  const spine = `${s1[0]},${s1[1]} ${s2[0]},${s2[1]}`;

  const stub = (s: { x: number; y: number }) => {
    const a = ISO(junctionX, s.y, zTrace);
    const b = ISO(s.x, s.y, zTrace);
    return `${a[0]},${a[1]} ${b[0]},${b[1]}`;
  };
  const stubs = [ssp1, ssp2, ssp3].map(stub);

  return (
    <g>
      <g fill="none" stroke="rgba(155,138,255,0.65)" strokeWidth="1">
        <polyline points={snapTrace} />
        <polyline points={trunk} />
        <polyline points={spine} />
        {stubs.map((s, i) => (
          <polyline key={'stub' + i} points={s} />
        ))}
      </g>

      <IsoSSP x={ssp1.x} y={ssp1.y} label="SSP·1" />
      <IsoSSP x={ssp2.x} y={ssp2.y} label="SSP·2" />
      <IsoSSP x={ssp3.x} y={ssp3.y} label="SSP·3" />
      <IsoSnapshot x={snapshot.x} y={snapshot.y} />
      <IsoScheduler x={scheduler.x} y={scheduler.y} />
    </g>
  );
}

export function HeroStack() {
  const [shown, setShown] = useState<[boolean, boolean, boolean]>([false, false, false]);

  useEffect(() => {
    const timers = [
      setTimeout(() => setShown((s) => [s[0], s[1], true]), 120),
      setTimeout(() => setShown((s) => [s[0], true, s[2]]), 360),
      setTimeout(() => setShown((s) => [true, s[1], s[2]]), 600),
    ];
    return () => timers.forEach(clearTimeout);
  }, []);

  const layerClass = (i: number) => `hero-layer${shown[i] ? ' hero-layer-in' : ''}`;

  return (
    <div className="hero-stack">
      <svg viewBox="0 0 560 620" width="100%" preserveAspectRatio="xMidYMid meet" className="hero-stack-svg">
        <defs>
          <linearGradient id="platTop" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="rgba(255,255,255,0.06)" />
            <stop offset="1" stopColor="rgba(255,255,255,0.015)" />
          </linearGradient>
          <linearGradient id="platPurple" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="rgba(124,106,239,0.22)" />
            <stop offset="1" stopColor="rgba(124,106,239,0.04)" />
          </linearGradient>
          <radialGradient id="discGlow" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="rgba(155,138,255,0.45)" />
            <stop offset="1" stopColor="rgba(124,106,239,0)" />
          </radialGradient>
          <linearGradient id="socTop" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1a1420" />
            <stop offset="0.5" stopColor="#0d0812" />
            <stop offset="1" stopColor="#05030a" />
          </linearGradient>
          <linearGradient id="socSideL" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#15101c" />
            <stop offset="1" stopColor="#040208" />
          </linearGradient>
          <linearGradient id="socSideR" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor="#0e0a16" />
            <stop offset="1" stopColor="#03020a" />
          </linearGradient>
          <radialGradient id="socGlow" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="rgba(198,140,255,0.9)" />
            <stop offset="0.5" stopColor="rgba(124,106,239,0.55)" />
            <stop offset="1" stopColor="rgba(80,120,255,0)" />
          </radialGradient>
          <radialGradient id="socGlowPink" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="rgba(255,130,200,0.85)" />
            <stop offset="0.6" stopColor="rgba(198,120,255,0.4)" />
            <stop offset="1" stopColor="rgba(120,80,255,0)" />
          </radialGradient>
          <filter id="engrave" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur in="SourceAlpha" stdDeviation="0.35" result="blurA" />
            <feOffset in="blurA" dx="0.3" dy="0.4" result="offA" />
            <feComposite in="offA" in2="SourceAlpha" operator="arithmetic" k2={-1} k3={1} result="innerShadow" />
            <feGaussianBlur in="SourceAlpha" stdDeviation="0.25" result="blurB" />
            <feOffset in="blurB" dx="-0.3" dy="-0.35" result="offB" />
            <feComposite in="offB" in2="SourceAlpha" operator="arithmetic" k2={-1} k3={1} result="innerHighlight" />
            <feFlood floodColor="#f4efff" floodOpacity="0.55" result="highlightColor" />
            <feComposite in="highlightColor" in2="innerHighlight" operator="in" result="coloredHighlight" />
            <feMerge>
              <feMergeNode in="SourceGraphic" />
              <feMergeNode in="innerShadow" />
              <feMergeNode in="coloredHighlight" />
            </feMerge>
          </filter>
          <linearGradient id="metalFill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a3244" />
            <stop offset="0.45" stopColor="#6b6078" />
            <stop offset="0.55" stopColor="#8a7ea0" />
            <stop offset="1" stopColor="#2a2434" />
          </linearGradient>
          <filter id="softshadow" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur in="SourceAlpha" stdDeviation="6" />
            <feOffset dy="6" />
            <feComponentTransfer>
              <feFuncA type="linear" slope="0.45" />
            </feComponentTransfer>
            <feMerge>
              <feMergeNode />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
        </defs>

        {/* Bottom: sp00ky engine */}
        <g transform="translate(280 498)">
          <g className={layerClass(2)}>
            <polygon
              points="0,-120 240,0 0,120 -240,0"
              fill="url(#platPurple)"
              stroke="rgba(155,138,255,0.55)"
              strokeWidth="1.2"
            />
            <ellipse cx="0" cy="0" rx="110" ry="55" fill="url(#discGlow)" />
            <g transform="translate(0 -12)" filter="url(#softshadow)">
              <SpookyEngine />
            </g>
            <g transform="matrix(1 0.5 -1 0.5 0 0)" style={{ opacity: 0.85 }}>
              <text
                x="-95"
                y="90"
                fill="rgba(155,138,255,0.85)"
                fontFamily="JetBrains Mono, monospace"
                fontSize="11"
                fontWeight="700"
                letterSpacing="0.22em"
              >
                SP00KY
              </text>
              <text
                x="-95"
                y="108"
                fill="rgba(255,255,255,0.45)"
                fontFamily="JetBrains Mono, monospace"
                fontSize="9"
                letterSpacing="0.08em"
              >
                RUST SYNC ENGINE
              </text>
            </g>
          </g>
        </g>

        {/* Middle: SurrealDB */}
        <g transform="translate(280 328)">
          <g className={layerClass(1)}>
            <polygon
              points="0,-120 240,0 0,120 -240,0"
              fill="url(#platTop)"
              stroke="rgba(255,255,255,0.22)"
              strokeWidth="1.2"
            />

            {/* SurrealDB logo mapped flat onto the isometric plane */}
            <g transform="matrix(1 0.5 -1 0.5 0 0)">
              <ellipse cx="0" cy="0" rx="95" ry="95" fill="rgba(200,120,255,0.08)" />
              <g style={{ opacity: 0.6, filter: 'saturate(0.7) brightness(1.1)' }}>
                <image
                  href="/surrealdb-logo.png"
                  x="-85"
                  y="-98"
                  width="170"
                  height="196"
                  preserveAspectRatio="xMidYMid meet"
                />
              </g>
            </g>

            <g transform="matrix(1 0.5 -1 0.5 0 0)" style={{ opacity: 0.8 }}>
              <text
                x="-90"
                y="85"
                fill="#ffffff"
                fontFamily="JetBrains Mono, monospace"
                fontSize="11"
                fontWeight="700"
                letterSpacing="0.22em"
              >
                SURREALDB
              </text>
            </g>
          </g>
        </g>

        {/* Top: Frontend */}
        <g transform="translate(280 158)">
          <g className={layerClass(0)}>
            <polygon
              points="0,-120 240,0 0,120 -240,0"
              fill="url(#platTop)"
              stroke="rgba(255,255,255,0.22)"
              strokeWidth="1.2"
            />
            <g transform="translate(-14 -36)" filter="url(#softshadow)">
              <IsoWindow />
            </g>
            <g transform="translate(80 10)" filter="url(#softshadow)">
              <IsoMobile />
            </g>
            <g transform="matrix(1 0.5 -1 0.5 0 0)" style={{ opacity: 0.8 }}>
              <text
                x="-90"
                y="85"
                fill="#ffffff"
                fontFamily="JetBrains Mono, monospace"
                fontSize="11"
                fontWeight="700"
                letterSpacing="0.22em"
              >
                FRONTEND
              </text>
            </g>
          </g>
        </g>
      </svg>
    </div>
  );
}

export default HeroStack;
