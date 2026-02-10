import React from 'react';

export const HeroTerminal: React.FC = () => {
  return (
    <div className="hero-terminal relative w-full rounded-xl border border-surface-border bg-surface-elevated/50 hover:bg-surface-elevated/95 backdrop-blur-md shadow-2xl overflow-hidden transition-all duration-300">
      {/* Window Chrome */}
      <div className="terminal-chrome flex items-center justify-between bg-surface-elevated/60 hover:bg-surface-elevated/100 backdrop-blur-sm border-b border-surface-border px-4 py-3 transition-all duration-300">
        <div className="flex items-center gap-3">
          <div className="traffic-lights flex gap-1.5">
            <span className="dot w-3 h-3 rounded-full bg-red-500/50"></span>
            <span className="dot w-3 h-3 rounded-full bg-yellow-500/50"></span>
            <span className="dot w-3 h-3 rounded-full bg-accent-500/50"></span>
          </div>
          <span className="terminal-title text-text-tertiary text-xs">queries.ts</span>
        </div>
      </div>

      {/* Terminal Content */}
      <div className="terminal-content relative z-10 bg-surface/50 hover:bg-surface/95 backdrop-blur-sm p-6 transition-all duration-300">
        <pre className="font-mono text-sm text-text-secondary leading-relaxed whitespace-pre-wrap break-words">
          <code>
            <span className="text-purple-400 font-bold">import</span> {'{'} useQuery {'}'} <span className="text-purple-400 font-bold">from</span> <span className="text-accent-400">&quot;@spooky/client-react&quot;</span>;
            {'\n'}<span className="text-purple-400 font-bold">import</span> {'{'} db {'}'} <span className="text-purple-400 font-bold">from</span> <span className="text-accent-400">&quot;../db&quot;</span>;
            {'\n'}
            {'\n'}<span className="text-brand-400 font-bold">const</span> <span className="text-yellow-400">ThreadList</span> = () =&gt; {'{'}
            {'\n'}  <span className="text-brand-400 font-bold">const</span> result = <span className="text-yellow-400">useQuery</span>(db, () =&gt;
            {'\n'}    db.<span className="text-yellow-400">query</span>(<span className="text-accent-400">&quot;thread&quot;</span>)
            {'\n'}      .<span className="text-yellow-400">related</span>(<span className="text-accent-400">&quot;author&quot;</span>)
            {'\n'}      .<span className="text-yellow-400">related</span>(<span className="text-accent-400">&quot;comments&quot;</span>)
            {'\n'}      .<span className="text-yellow-400">orderBy</span>(<span className="text-accent-400">&quot;created_at&quot;</span>, <span className="text-accent-400">&quot;desc&quot;</span>)
            {'\n'}      .<span className="text-yellow-400">limit</span>(<span className="text-green-400">10</span>)
            {'\n'}  );
            {'\n'}
            {'\n'}  <span className="text-purple-400 font-bold">return</span> result.data.<span className="text-yellow-400">map</span>(thread =&gt; (
            {'\n'}    <span className="text-gray-500">&lt;</span><span className="text-blue-400">div</span> <span className="text-yellow-400">key</span>=<span className="text-gray-500">{'{'}</span>thread.id<span className="text-gray-500">{'}'}</span><span className="text-gray-500">&gt;</span>
            {'\n'}      <span className="text-gray-500">{'{'}</span>thread.title<span className="text-gray-500">{'}'}</span>
            {'\n'}    <span className="text-gray-500">&lt;/</span><span className="text-blue-400">div</span><span className="text-gray-500">&gt;</span>
            {'\n'}  ));
            {'\n'}{'}'};
          </code>
        </pre>
      </div>

      {/* Gradient Effect */}
      <div className="terminal-gradient absolute bottom-0 right-0 w-64 h-64 pointer-events-none z-0">
        <div
          className="absolute inset-0 rounded-full blur-3xl"
          style={{
            background: 'radial-gradient(circle, rgba(138, 89, 255, 0.15) 0%, transparent 70%)',
          }}
        ></div>
      </div>
    </div>
  );
};
