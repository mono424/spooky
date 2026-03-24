import React from 'react';
import { SchemaFilePreview } from './SchemaFilePreview';
import { LanguageOutputTabs } from './LanguageOutputTabs';

export const SchemaWorkflowEyecatcher: React.FC = () => {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-16 items-start">
      {/* Left: Schema file + arrow + generated output tabs */}
      <div className="order-2 lg:order-1 space-y-3">
        <SchemaFilePreview />

        {/* Arrow indicating transformation */}
        <div className="flex items-center justify-center py-1">
          <div className="relative">
            <div className="absolute inset-0 bg-brand-500/20 blur-lg rounded-full animate-pulse"></div>
            <svg
              className="w-6 h-6 text-brand-500 relative z-10"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 14l-7 7m0 0l-7-7m7 7V3"
              />
            </svg>
          </div>
        </div>

        <LanguageOutputTabs />
      </div>

      {/* Right: Description sidebar */}
      <div className="order-1 lg:order-2 space-y-8">
        <div>
          <div className="inline-block border border-brand-500/30 bg-brand-500/10 px-3 py-1.5 rounded-lg text-xs font-medium text-brand-400 mb-4">
            Type Safety
          </div>
          <h3 className="text-2xl font-bold text-text-primary mb-4">Schema-First Development</h3>
          <p className="text-text-tertiary text-body leading-relaxed">
            Define your database schema once in SurrealDB, and let Sp00ky generate type-safe
            clients for all your targets. One source of truth for your entire stack.
          </p>
        </div>

        <div className="space-y-6">
          {/* Multi-Target Codegen */}
          <div className="group">
            <h4 className="text-text-primary font-semibold mb-2 group-hover:text-brand-400 transition-colors flex items-center gap-2">
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="m18 16 4-4-4-4" />
                <path d="m6 8-4 4 4 4" />
                <path d="m14.5 4-5 16" />
              </svg>
              Multi-Target Codegen
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              Generate TypeScript, Dart, and Zod clients from a single schema definition. Every
              target stays in sync automatically.
            </p>
          </div>

          {/* End-to-End Type Safety */}
          <div className="group">
            <h4 className="text-text-primary font-semibold mb-2 group-hover:text-accent-400 transition-colors flex items-center gap-2">
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10" />
                <path d="m9 12 2 2 4-4" />
              </svg>
              End-to-End Type Safety
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              Types flow from your database schema to your application code. Catch errors at compile
              time, not in production.
            </p>
          </div>

          {/* CLI Integration */}
          <div className="group">
            <h4 className="text-text-primary font-semibold mb-2 group-hover:text-brand-400 transition-colors flex items-center gap-2">
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polyline points="4 17 10 11 4 5" />
                <line x1="12" x2="20" y1="19" y2="19" />
              </svg>
              CLI Integration
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              Run <code className="bg-surface px-1.5 py-0.5 rounded text-text-secondary font-mono text-sm">sp00ky generate</code> to
              regenerate clients whenever your schema changes. Fits right into your workflow.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};
