import React from 'react';

interface ProjectExplorerProps {
  variant: 'compact' | 'full';
  highlightedFiles?: string[];
}

export default function ProjectExplorer({ variant, highlightedFiles = [] }: ProjectExplorerProps) {
  const isHighlighted = (file: string) => highlightedFiles.includes(file);

  if (variant === 'compact') {
    return (
      <div className="bg-surface/50 backdrop-blur-sm border border-surface-border rounded-lg p-3 h-full sticky top-24">
        <div className="text-xs font-semibold text-text-primary mb-4">
          Project Files
        </div>
        <div className="font-mono text-xs space-y-1.5">
          {/* Flat file list with minimal styling */}
          <div className="flex items-center gap-2 text-text-muted transition-colors">
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>apps</span>
          </div>

          <div className="flex items-center gap-2 text-text-muted pl-4 transition-colors">
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>solid-app</span>
          </div>

          <div
            className={`flex items-center gap-2 pl-8 transition-colors ${
              isHighlighted('schema.gen.ts')
                ? 'text-brand-400 bg-brand-500/10 -mx-2 px-2 rounded'
                : 'text-text-muted'
            }`}
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
            </svg>
            <span>schema.gen.ts</span>
          </div>

          <div className="flex items-center gap-2 text-text-muted transition-colors">
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>packages</span>
          </div>

          <div className="flex items-center gap-2 text-text-muted pl-4 transition-colors">
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>schema</span>
          </div>

          <div
            className={`flex items-center gap-2 pl-8 transition-colors ${
              isHighlighted('schema.surql')
                ? 'text-accent-400 bg-accent-500/10 -mx-2 px-2 rounded'
                : 'text-text-muted'
            }`}
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
            </svg>
            <span>schema.surql</span>
          </div>
        </div>
      </div>
    );
  }

  // Full variant with expandable tree (for future use)
  return (
    <div className="bg-surface/50 backdrop-blur-sm border border-surface-border rounded-lg p-6 h-full">
      <div className="text-sm font-semibold text-text-primary mb-6">
        Project Explorer
      </div>
      <div className="font-mono text-sm space-y-1">
        <div className="text-text-tertiary flex items-center gap-2">
          <span className="text-text-muted">▾</span>
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
          </svg>
          <span>apps</span>
        </div>
        <div className="pl-4 text-text-muted border-l border-surface-border ml-1">
          <div className="py-1 cursor-pointer transition-colors flex items-center gap-2">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>solid-app</span>
          </div>
          <div className="pl-4 border-l border-surface-border ml-1">
            <div className="py-1 cursor-pointer transition-colors flex items-center gap-2">
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
              </svg>
              <span>src</span>
            </div>
            <div
              className={`py-1 flex items-center gap-2 ${
                isHighlighted('schema.gen.ts') ? 'text-brand-400' : 'text-text-muted'
              }`}
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
              </svg>
              <span>schema.gen.ts</span>
            </div>
          </div>
        </div>

        <div className="text-text-tertiary flex items-center gap-2 mt-2">
          <span className="text-text-muted">▾</span>
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
          </svg>
          <span>packages</span>
        </div>
        <div className="pl-4 border-l border-surface-border ml-1">
          <div className="py-1 text-text-muted flex items-center gap-2">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span>schema</span>
          </div>
          <div className="pl-4 border-l border-surface-border ml-1">
            <div
              className={`py-1 flex items-center gap-2 ${
                isHighlighted('schema.surql')
                  ? 'text-accent-400 bg-accent-500/10 -mx-2 px-2 rounded'
                  : 'text-text-muted'
              }`}
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
              </svg>
              <span>schema.surql</span>
            </div>
          </div>
        </div>
      </div>

      <div
        className="mt-8 text-xs text-text-tertiary border-t border-surface-border pt-4 leading-relaxed"
      >
        <span className="text-accent-500 font-bold">TIP:</span> Add a pre-commit hook to automatically
        update all types on schema change.
      </div>
    </div>
  );
}
