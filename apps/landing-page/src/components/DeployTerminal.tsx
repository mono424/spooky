import React from 'react';

const services = [
  { name: 'Database', detail: 'SurrealDB v2.3' },
  { name: 'Scheduler', detail: '1 cron job active' },
  { name: 'SSP Module 1', detail: 'us-east-1' },
  { name: 'SSP Module 2', detail: 'eu-west-1' },
  { name: 'SSP Module 3', detail: 'ap-south-1' },
  { name: 'Backend', detail: '4 functions' },
];

const issues = [
  { id: 'ACM-142', title: 'Add real-time presence indicators', status: 'In Progress', priority: 3, assignee: 'S', label: 'Feature' },
  { id: 'ACM-141', title: 'Fix sync conflict on concurrent edits', status: 'In Progress', priority: 3, assignee: 'M', label: 'Bug' },
  { id: 'ACM-140', title: 'Migrate user settings to new schema', status: 'Todo', priority: 2, assignee: 'P', label: 'Backend' },
  { id: 'ACM-139', title: 'Implement offline queue retry logic', status: 'Todo', priority: 2, assignee: 'J', label: 'Feature' },
  { id: 'ACM-138', title: 'Dashboard performance regression', status: 'Todo', priority: 3, assignee: 'S', label: 'Bug' },
  { id: 'ACM-137', title: 'Add batch export for analytics data', status: 'Backlog', priority: 1, assignee: 'M', label: 'Feature' },
  { id: 'ACM-136', title: 'Update onboarding copy and illustrations', status: 'Backlog', priority: 1, assignee: 'P', label: 'Design' },
  { id: 'ACM-135', title: 'Refactor auth token refresh flow', status: 'Backlog', priority: 2, assignee: 'J', label: 'Backend' },
  { id: 'ACM-134', title: 'Add keyboard shortcuts for navigation', status: 'Backlog', priority: 1, assignee: 'S', label: 'Feature' },
];

const PriorityIcon: React.FC<{ level: number }> = ({ level }) => (
  <div className="flex items-center gap-[2px]">
    {[0, 1, 2, 3].map((i) => (
      <div
        key={i}
        className={`w-[3px] rounded-sm ${i < level ? 'bg-white/40' : 'bg-white/10'}`}
        style={{ height: `${6 + i * 2}px` }}
      />
    ))}
  </div>
);

export const DeployTerminal: React.FC = () => {
  return (
    <div className="relative w-full max-w-4xl mx-auto" style={{ perspective: '1800px' }}>
      {/* Glow */}
      <div
        className="absolute -bottom-12 left-1/2 -translate-x-1/2 w-3/4 h-40 rounded-full blur-3xl opacity-10"
        style={{ background: 'radial-gradient(ellipse, rgba(255, 255, 255, 0.15), transparent)' }}
      />

      {/* ── BACKGROUND: Linear-style issue tracker ── */}
      <div
        className="relative rounded-xl border border-white/[0.06] bg-[#0c0c0c] overflow-hidden opacity-50"
        style={{
          transform: 'scale(0.88) translateX(16%) translateY(3%) rotateY(-5deg) rotateX(4deg)',
          transformOrigin: 'center right',
          boxShadow: '0 30px 60px -20px rgba(0, 0, 0, 0.5)',
        }}
      >
        {/* Browser chrome */}
        <div className="flex items-center gap-3 bg-[#141414] border-b border-white/[0.04] px-4 py-2">
          <div className="flex gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.12]" />
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.12]" />
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.12]" />
          </div>
          <div className="flex-1 flex justify-center">
            <div className="flex items-center gap-2 bg-white/[0.03] border border-white/[0.05] rounded-md px-3 py-1 max-w-xs w-full">
              <svg className="w-3 h-3 text-white/20 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
              </svg>
              <span className="text-[11px] text-white/30 font-mono truncate">acme.spky.cloud</span>
            </div>
          </div>
          <div className="w-16" />
        </div>

        {/* App shell */}
        <div className="flex min-h-[480px] md:min-h-[540px]">
          {/* Sidebar */}
          <div className="w-48 shrink-0 border-r border-white/[0.04] py-3 px-2 hidden md:flex flex-col">
            <div className="flex items-center gap-2.5 px-2.5 mb-5">
              <div className="w-6 h-6 rounded-md bg-white/[0.08] flex items-center justify-center">
                <span className="text-white/50 text-[10px] font-bold">A</span>
              </div>
              <span className="text-[13px] font-semibold text-white/70 tracking-tight">Acme</span>
            </div>

            <div className="space-y-0.5 mb-4">
              {[
                { icon: 'M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6', label: 'Inbox', count: 3 },
                { icon: 'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2', label: 'My Issues', count: 5, active: true },
                { icon: 'M4 6h16M4 10h16M4 14h16M4 18h16', label: 'All Issues' },
              ].map((item) => (
                <div key={item.label} className={`flex items-center gap-2.5 px-2.5 py-1.5 rounded-md text-[12px] ${item.active ? 'bg-white/[0.05] text-white/70' : 'text-white/30'}`}>
                  <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d={item.icon} />
                  </svg>
                  <span className="flex-1">{item.label}</span>
                  {item.count && <span className="text-[10px] text-white/25 bg-white/[0.04] px-1.5 py-0.5 rounded">{item.count}</span>}
                </div>
              ))}
            </div>

            <div className="text-[10px] text-white/20 uppercase tracking-wider px-2.5 mb-2 mt-2">Teams</div>
            <div className="space-y-0.5 mb-4">
              {['Engineering', 'Design', 'Growth'].map((team) => (
                <div key={team} className="flex items-center gap-2.5 px-2.5 py-1.5 rounded-md text-[12px] text-white/30">
                  <span className="w-3.5 h-3.5 rounded bg-white/[0.05] flex items-center justify-center text-[8px] font-medium shrink-0">{team[0]}</span>
                  {team}
                </div>
              ))}
            </div>

            <div className="text-[10px] text-white/20 uppercase tracking-wider px-2.5 mb-2 mt-2">Views</div>
            <div className="space-y-0.5">
              {['Active Cycle', 'Backlog', 'Board'].map((view) => (
                <div key={view} className="flex items-center gap-2.5 px-2.5 py-1.5 rounded-md text-[12px] text-white/30">
                  <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z" />
                  </svg>
                  {view}
                </div>
              ))}
            </div>
          </div>

          {/* Main content */}
          <div className="flex-1 flex flex-col">
            {/* Toolbar */}
            <div className="flex items-center justify-between px-4 py-2.5 border-b border-white/[0.04]">
              <div className="flex items-center gap-3">
                <span className="text-[13px] font-medium text-white/60">My Issues</span>
                <span className="text-[11px] text-white/25 bg-white/[0.04] px-1.5 py-0.5 rounded">{issues.length}</span>
              </div>
              <div className="flex items-center gap-2">
                <div className="flex items-center gap-1 text-[11px] text-white/25 bg-white/[0.02] border border-white/[0.05] rounded-md px-2 py-1">
                  <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z" />
                  </svg>
                  Filter
                </div>
                <div className="flex items-center gap-1 text-[11px] text-white/25 bg-white/[0.02] border border-white/[0.05] rounded-md px-2 py-1">
                  <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M3 7h18M7 12h10M10 17h4" />
                  </svg>
                  Group
                </div>
              </div>
            </div>

            {/* Issue rows */}
            <div className="flex-1">
              {issues.map((issue, i) => (
                <div key={issue.id} className={`flex items-center gap-3 px-4 py-2.5 border-b border-white/[0.03] text-[12px] ${i === 0 ? 'bg-white/[0.015]' : ''}`}>
                  <PriorityIcon level={issue.priority} />
                  <div className="w-4 h-4 rounded-full border-[1.5px] border-white/[0.12] shrink-0 flex items-center justify-center">
                    {issue.status === 'In Progress' && <div className="w-1.5 h-1.5 rounded-full bg-white/30" />}
                  </div>
                  <span className="text-white/25 w-16 shrink-0 font-mono text-[11px]">{issue.id}</span>
                  <span className="text-white/60 flex-1 truncate">{issue.title}</span>
                  <span className="text-[10px] text-white/20 bg-white/[0.04] border border-white/[0.06] px-1.5 py-0.5 rounded shrink-0">{issue.label}</span>
                  <div className="w-5 h-5 rounded-full bg-white/[0.05] flex items-center justify-center text-[9px] font-medium text-white/30 shrink-0">{issue.assignee}</div>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>

      {/* ── FOREGROUND: Terminal window ── */}
      <div
        className="absolute bottom-12 -left-2 md:-left-6 w-[90%] md:w-[65%] z-20 rounded-xl border border-white/[0.12] bg-[#0a0a0a] overflow-hidden"
        style={{
          boxShadow: '0 40px 100px -10px rgba(0, 0, 0, 0.9), 0 0 0 1px rgba(255,255,255,0.05)',
        }}
      >
        {/* Terminal chrome */}
        <div className="flex items-center gap-3 bg-[#141414] border-b border-white/[0.06] px-4 py-2.5">
          <div className="flex gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.1]" />
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.1]" />
            <span className="w-2.5 h-2.5 rounded-full bg-white/[0.1]" />
          </div>
          <span className="text-white/25 text-[11px] font-mono">~/acme-app</span>
        </div>

        {/* Terminal body */}
        <div className="px-5 py-4 font-mono text-[12px] md:text-[13px] leading-[1.9] text-left">
          <div>
            <span className="text-text-muted select-none">$ </span>
            <span className="text-text-primary font-medium">npx @spky/cli deploy</span>
          </div>
          <div className="h-3" />
          <div className="text-text-tertiary">Deploying to sp00ky cloud...</div>
          <div className="h-3" />
          {services.map((s) => (
            <div key={s.name}>
              <span className="text-accent-400">  ✓</span>
              <span className="text-text-secondary">{`  ${s.name.padEnd(14)}`}</span>
              <span className="text-accent-400/70">{'deployed   '}</span>
              <span className="text-text-muted">{s.detail}</span>
            </div>
          ))}
          <div className="h-3" />
          <div className="text-white/[0.07] select-none">  ─────────────────────────────────────────</div>
          <div className="h-3" />
          <div className="text-accent-400 font-medium">  All services deployed successfully.</div>
          <div>
            <span className="text-text-muted">  App available at </span>
            <span className="text-brand-400 underline underline-offset-2 decoration-brand-400/30">https://acme.spky.cloud</span>
          </div>
        </div>
      </div>
    </div>
  );
};
