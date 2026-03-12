import React from 'react';

export const HeroTerminal: React.FC = () => {
  return (
    <div className="hero-terminal relative w-full rounded-xl border border-white/10 bg-surface-elevated overflow-hidden transition-all duration-300">
      {/* Window Chrome */}
      <div className="terminal-chrome flex items-center justify-between bg-surface-elevated border-b border-white/10 px-4 py-3 transition-all duration-300">
        <div className="flex items-center gap-3">
          <div className="traffic-lights flex gap-1.5">
            <span className="dot w-3 h-3 rounded-full bg-white/15"></span>
            <span className="dot w-3 h-3 rounded-full bg-white/15"></span>
            <span className="dot w-3 h-3 rounded-full bg-white/15"></span>
          </div>
          <span className="terminal-title text-text-tertiary text-xs">queries.ts</span>
        </div>
      </div>

      {/* Terminal Content */}
      <div className="terminal-content relative z-10 bg-surface p-6 transition-all duration-300">
        <pre className="font-mono text-sm text-white/60 leading-relaxed whitespace-pre-wrap break-words">
          <code>
            <span className="text-brand-400/70">import</span> {'{'} useQuery {'}'} <span className="text-brand-400/70">from</span> <span className="text-white/65">&quot;@spooky-sync/client-solid&quot;</span>;
            {'\n'}<span className="text-brand-400/70">import</span> {'{'} db {'}'} <span className="text-brand-400/70">from</span> <span className="text-white/65">&quot;../db&quot;</span>;
            {'\n'}
            {'\n'}<span className="text-brand-400/70">const</span> <span className="text-white/85">ThreadList</span> = () =&gt; {'{'}
            {'\n'}  <span className="text-brand-400/70">const</span> result = <span className="text-white/85">useQuery</span>(db, () =&gt;
            {'\n'}    db.<span className="text-white/85">query</span>(<span className="text-white/65">&quot;thread&quot;</span>)
            {'\n'}      .<span className="text-white/85">related</span>(<span className="text-white/65">&quot;author&quot;</span>)
            {'\n'}      .<span className="text-white/85">related</span>(<span className="text-white/65">&quot;comments&quot;</span>)
            {'\n'}      .<span className="text-white/85">orderBy</span>(<span className="text-white/65">&quot;created_at&quot;</span>, <span className="text-white/65">&quot;desc&quot;</span>)
            {'\n'}      .<span className="text-white/85">limit</span>(<span className="text-white/65">10</span>)
            {'\n'}      .<span className="text-white/85">build</span>()
            {'\n'}  );
            {'\n'}
            {'\n'}  <span className="text-brand-400/70">return</span> (
            {'\n'}    <span className="text-white/45">&lt;</span><span className="text-white/60">For</span> <span className="text-white/60">each</span>=<span className="text-white/45">{'{'}</span>result.<span className="text-white/85">data</span>()<span className="text-white/45">{'}'}</span><span className="text-white/45">&gt;</span>
            {'\n'}      <span className="text-white/45">{'{'}</span>(thread) =&gt; (
            {'\n'}        <span className="text-white/45">&lt;</span><span className="text-white/60">div</span><span className="text-white/45">&gt;</span><span className="text-white/45">{'{'}</span>thread.title<span className="text-white/45">{'}'}</span><span className="text-white/45">&lt;/</span><span className="text-white/60">div</span><span className="text-white/45">&gt;</span>
            {'\n'}      ){'}'}
            {'\n'}    <span className="text-white/45">&lt;/</span><span className="text-white/60">For</span><span className="text-white/45">&gt;</span>
            {'\n'}  );
            {'\n'}{'}'};
          </code>
        </pre>
      </div>
    </div>
  );
};
