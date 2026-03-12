import React from 'react';

interface DXFeatureBoxProps {
  figure: string;
  title: string;
  description: string;
  children: React.ReactNode;
  className?: string;
}

export const DXFeatureBox: React.FC<DXFeatureBoxProps> = ({
  figure,
  title,
  description,
  children,
  className = '',
}) => {
  return (
    <div
      className={`bg-[#0a0a0a] overflow-hidden flex flex-col min-h-[560px] ${className}`}
    >
      {/* FIG label */}
      <div className="px-6 pt-5 pb-2">
        <span className="font-mono text-[11px] text-text-muted tracking-wider uppercase">
          {figure}
        </span>
      </div>

      {/* Visual content area with fade edges */}
      <div className="flex-1 px-4 overflow-hidden relative">
        <div className="h-full flex items-center justify-center">
          {children}
        </div>
        {/* Bottom fade */}
        <div
          className="absolute inset-x-0 bottom-0 h-12 pointer-events-none"
          style={{ background: 'linear-gradient(to top, #0a0a0a, transparent)' }}
        />
        {/* Top fade */}
        <div
          className="absolute inset-x-0 top-0 h-32 pointer-events-none"
          style={{ background: 'linear-gradient(to bottom, #0a0a0a 10%, transparent)' }}
        />
        {/* Left fade */}
        <div
          className="absolute inset-y-0 left-0 w-8 pointer-events-none"
          style={{ background: 'linear-gradient(to right, #0a0a0a, transparent)' }}
        />
        {/* Right fade */}
        <div
          className="absolute inset-y-0 right-0 w-8 pointer-events-none"
          style={{ background: 'linear-gradient(to left, #0a0a0a, transparent)' }}
        />
      </div>

      {/* Bottom text */}
      <div className="px-6 pb-6 pt-4">
        <h3 className="text-lg font-semibold text-text-primary mb-1">{title}</h3>
        <p className="text-sm text-text-tertiary leading-relaxed">{description}</p>
      </div>
    </div>
  );
};
