import React from 'react';
import { SchemaFilePreview } from './SchemaFilePreview';
import { LanguageOutputTabs } from './LanguageOutputTabs';

export const SchemaConversionBox: React.FC = () => {
  return (
    <div className="w-full flex flex-col gap-3 py-2" style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
      <SchemaFilePreview />

      {/* Arrow */}
      <div className="flex items-center justify-center text-text-muted">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" />
          <polyline points="19 12 12 19 5 12" />
        </svg>
      </div>

      <LanguageOutputTabs />
    </div>
  );
};
