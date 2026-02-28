import React from 'react';
import { SchemaFilePreview } from './SchemaFilePreview';
import { GeneratedOutputGrid } from './GeneratedOutputGrid';
import { TerminalCommand } from './TerminalCommand';
import { LanguageOutputTabs } from './LanguageOutputTabs';

export const SchemaWorkflowEyecatcher: React.FC = () => {
  return (
    <details className="schema-details mt-12 max-w-7xl mx-auto border border-surface-border/30 rounded-2xl bg-surface/50 backdrop-blur-sm overflow-hidden transition-all duration-400 shadow-lg shadow-black/20">
      <summary className="list-none cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500 p-6 lg:p-8 hover:bg-surface/80 transition-colors">

        {/* Collapsed State */}
        <div className="flex flex-col lg:flex-row gap-8 lg:gap-12 items-center">
          {/* Left: Schema Preview */}
          <div className="w-full lg:w-1/3 flex-shrink-0">
            <SchemaFilePreview />
          </div>

          {/* Center: Animated Arrow */}
          <div className="flex-shrink-0">
            <div className="relative">
              <div className="absolute inset-0 bg-brand-500/20 blur-xl rounded-full animate-pulse"></div>
              <svg
                className="w-10 h-10 text-brand-500 relative z-10 rotate-90 lg:rotate-0 transition-transform duration-300"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 7l5 5m0 0l-5 5m5-5H6"
                />
              </svg>
            </div>
          </div>

          {/* Right: Generated Outputs */}
          <div className="w-full lg:flex-1">
            <GeneratedOutputGrid />
          </div>
        </div>

        {/* Expand CTA */}
        <div className="mt-8 pt-6 border-t border-surface-border/50 flex items-center justify-center gap-2 text-base text-text-tertiary hover:text-text-primary transition-colors">
          <span>Click to see generation in action</span>
          <svg
            className="w-5 h-5 transition-transform duration-300"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M19 9l-7 7-7-7"
            />
          </svg>
        </div>
      </summary>

      {/* Expanded State */}
      <div className="expanded-content border-t border-surface-border/50 bg-surface/30 p-6 lg:p-8">
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-8 lg:gap-12 items-start">
          {/* Left: Terminal Command */}
          <TerminalCommand />

          {/* Right: Language Output Tabs */}
          <LanguageOutputTabs />
        </div>
      </div>
    </details>
  );
};
