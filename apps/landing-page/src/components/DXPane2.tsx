import React from 'react';
import { DevToolsSimulation } from './DevToolsSimulation';

export const DXPane2: React.FC = () => {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-16 items-start">
      {/* Left: DevTools Simulation */}
      <div className="shadow-2xl">
        <DevToolsSimulation />
      </div>

      {/* Right: Complete description sidebar with ALL information */}
      <div className="space-y-8">
        <div>
          <div className="inline-block border border-brand-500/30 bg-brand-500/10 px-3 py-1.5 rounded-lg text-xs font-medium text-brand-400 mb-4">
            DevTools
          </div>
          <h3 className="text-2xl font-bold text-text-primary mb-4">Effortlessly Transparent</h3>
          <p className="text-text-tertiary text-body leading-relaxed">
            Stop guessing what your application is doing. Sp00ky provides a complete suite of
            tools embedded directly in your browser.
          </p>
        </div>

        <div className="space-y-6">
          {/* Live Inspection */}
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
                <path d="M21 11V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h6" />
                <path d="m12 12 4 10 1.7-4.3L22 16Z" />
              </svg>
              Live Inspection
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              View your local database state in real-time as it changes. No more console.logging
              state.
            </p>
          </div>

          {/* Query Monitor */}
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
                <path d="M12 2v4" />
                <path d="m16.2 7.8 2.9-2.9" />
                <path d="M18 12h4" />
                <path d="m16.2 16.2 2.9 2.9" />
                <path d="M12 18v4" />
                <path d="m4.9 19.1 2.9-2.9" />
                <path d="M2 12h4" />
                <path d="m4.9 4.9 2.9 2.9" />
              </svg>
              Query Monitor
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              Track active subscriptions, latency metrics, and data transfer sizes for
              optimization.
            </p>
          </div>

          {/* Events Timeline */}
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
                <circle cx="12" cy="12" r="10" />
                <polyline points="12 6 12 12 16 14" />
              </svg>
              Events Timeline
            </h4>
            <p className="text-text-tertiary leading-relaxed">
              Track every state change, network request, and query registration in a
              detailed, chronological timeline.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};
