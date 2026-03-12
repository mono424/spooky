import React from 'react';
import { DXFeatureBox } from './DXFeatureBox';
import { DevToolsSimulation } from './DevToolsSimulation';
import { SchemaLayerStack } from './SchemaLayerStack';

export const DXFeatureGrid: React.FC = () => {
  return (
    <div className="rounded-2xl border border-surface-border overflow-hidden bg-surface-border/50">
      <div className="grid grid-cols-1 md:grid-cols-2 gap-px">
        {/* Box 1: Schema-First Intelligence */}
        <DXFeatureBox
          figure="FIG. 3.1"
          title="Schema-First Intelligence"
          description="Define your schema once — get type-safe clients, context-aware autocomplete, and real-time validation across your entire stack."
          className="md:rounded-l-2xl"
        >
          <div className="w-full h-full flex items-center justify-center pt-8 pb-4 relative" style={{ paddingRight: 60 }}>
            <SchemaLayerStack />
            {/* Bottom fade */}
            <div
              className="absolute inset-x-0 bottom-0 h-28 pointer-events-none z-10"
              style={{ background: 'linear-gradient(to top, #0a0a0a 15%, transparent)' }}
            />
            {/* Right fade */}
            <div
              className="absolute inset-y-0 right-0 w-24 pointer-events-none z-10"
              style={{ background: 'linear-gradient(to left, #0a0a0a 10%, transparent)' }}
            />
          </div>
        </DXFeatureBox>

        {/* Box 2: DevTools */}
        <DXFeatureBox
          figure="FIG. 3.2"
          title="Effortlessly Transparent"
          description="A complete suite of developer tools embedded directly in your browser. Inspect state, monitor queries, and track events."
          className="md:rounded-r-2xl"
        >
          <div className="w-full relative" style={{ perspective: 800, perspectiveOrigin: '50% 100%' }}>
            <div style={{ transform: 'rotateX(8deg) scale(0.82) translateY(-60px)', transformOrigin: 'bottom center' }}>
              <DevToolsSimulation />
            </div>
          </div>
        </DXFeatureBox>
      </div>
    </div>
  );
};
