import { useState, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { ScrollRevealText } from './ScrollRevealText';

const features = [
  {
    fig: '0.1',
    title: 'Rust Core',
    subtitle: 'Memory-safe. No runtime overhead. Just raw performance.',
    icon: (
      <svg className="w-12 h-12" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth="0.75" strokeLinecap="round" strokeLinejoin="round">
        <rect x="4" y="4" width="16" height="16" rx="2" />
        <rect x="9" y="9" width="6" height="6" />
        <path d="M9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3" />
      </svg>
    ),
  },
  {
    fig: '0.2',
    title: 'Instant UI',
    subtitle: 'WASM powered optimistic updates. Every interaction feels immediate.',
    icon: (
      <svg className="w-12 h-12" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth="0.75" strokeLinecap="round" strokeLinejoin="round">
        <path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z" />
      </svg>
    ),
  },
  {
    fig: '0.3',
    title: 'Job Scheduler',
    subtitle: 'Retries, backoff, and outbox — your background work just runs.',
    icon: (
      <svg className="w-12 h-12" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth="0.75" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10" />
        <polyline points="12 6 12 12 16 14" />
      </svg>
    ),
  },
  {
    fig: '0.4',
    title: 'Fast Realtime',
    subtitle: 'Sub-50ms propagation. Changes arrive before you notice the delay.',
    icon: (
      <svg className="w-12 h-12" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth="0.75" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
      </svg>
    ),
  },
];

/** 4-column feature grid + "Local first" text — placed after the hero */
export function FeatureGrid() {
  return (
    <>
      <div className="max-w-3xl mb-16">
        <ScrollRevealText
          className="text-2xl md:text-3xl font-semibold leading-snug"
          segments={[
            { text: 'Local first. ', preRevealed: true },
            { text: 'Your app reads and writes to a local database. Users get instant responses, even without a connection. When they\'re back online, Spooky resolves changes and syncs state across every device — no loading spinners, no conflict modals, no extra code on your end.' },
          ]}
        />
      </div>

      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4">
        {features.map((feature, i) => (
          <div
            key={feature.fig}
            className={[
              'px-8 py-6',
              i !== 0 ? 'sm:border-l border-white/[0.06]' : '',
              i !== 0 ? 'border-t sm:border-t-0 border-white/[0.06]' : '',
              i === 2 ? 'sm:border-l-0 md:border-l' : '',
            ].join(' ')}
          >
            <div className="text-[11px] font-mono text-gray-600 uppercase tracking-wider mb-6">
              Fig {feature.fig}
            </div>
            <div className="flex items-center justify-center h-[120px] text-gray-600 mb-6">
              {feature.icon}
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
        <span className="text-[10px] font-mono font-medium tracking-wider uppercase text-gray-500 bg-white/[0.03] border border-white/[0.06] px-2.5 py-1 rounded-md">
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
        <div className="border border-dashed border-white/[0.06] p-3 rounded-lg bg-white/[0.02] relative mt-2">
          <div className="absolute -top-2 left-3 bg-[#0a0a0a] px-1.5 text-[8px] text-gray-600 font-medium uppercase tracking-wider">
            Server Infrastructure
          </div>

          <div className="flex flex-col gap-4">
            {/* Top row: SurrealDB — RPC — Scheduler */}
            <div className="flex gap-2 justify-center items-stretch">
              {/* SurrealDB */}
              <div className="border border-white/[0.06] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0">
                <div className="flex items-center justify-between border-b border-white/[0.06] pb-1">
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
              <div className="border border-white/[0.06] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0">
                <div className="flex items-center justify-between border-b border-white/[0.06] pb-1">
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
                <div key={ssp.name} className="border border-white/[0.06] bg-white/[0.02] p-1.5 rounded flex items-center justify-between">
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
      <ul className="space-y-2 font-mono text-xs text-gray-500 border-t border-white/[0.06] pt-4">
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
              className={`absolute right-0 top-0 bottom-0 w-full max-w-2xl bg-[#0a0a0a] border-l border-white/[0.06] transition-transform duration-300 ${drawerOpen ? 'translate-x-0' : 'translate-x-full'}`}
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
