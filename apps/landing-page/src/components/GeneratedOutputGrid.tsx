import React from 'react';
import { siTypescript, siDart, siZod } from 'simple-icons';

interface OutputCardProps {
  language: string;
  filename: string;
  color: 'blue' | 'teal' | 'purple';
  iconPath: string;
  iconColor: string;
}

const OutputCard: React.FC<OutputCardProps> = ({ language, filename, color, iconPath, iconColor }) => {
  const colorClasses = {
    blue: 'hover:border-blue-500/30',
    teal: 'hover:border-teal-500/30',
    purple: 'hover:border-purple-500/30',
  };

  return (
    <div
      className={`border border-surface-border bg-surface rounded-xl p-6 hover:-translate-y-1 transition-all duration-300 ${colorClasses[color]}`}
    >
      <div className="flex items-start justify-between mb-3">
        <svg
          role="img"
          viewBox="0 0 24 24"
          xmlns="http://www.w3.org/2000/svg"
          className="w-10 h-10"
          fill={iconColor}
        >
          <path d={iconPath} />
        </svg>
        <svg
          className="w-5 h-5 text-accent-500"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M5 13l4 4L19 7"
          />
        </svg>
      </div>
      <div className="font-semibold text-text-primary mb-1">{language}</div>
      <div className="font-mono text-xs text-text-tertiary">{filename}</div>
    </div>
  );
};

export const GeneratedOutputGrid: React.FC = () => {
  return (
    <div>
      <div className="text-sm text-text-tertiary text-center mb-4">Generated Files</div>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {/* TypeScript Card */}
        <OutputCard
          language="TypeScript"
          filename="schema.gen.ts"
          color="blue"
          iconPath={siTypescript.path}
          iconColor={`#${siTypescript.hex}`}
        />

        {/* Dart Card */}
        <OutputCard
          language="Dart"
          filename="schema.g.dart"
          color="teal"
          iconPath={siDart.path}
          iconColor={`#${siDart.hex}`}
        />

        {/* Zod Card */}
        <OutputCard
          language="Zod"
          filename="schema.zod.ts"
          color="purple"
          iconPath={siZod.path}
          iconColor={`#${siZod.hex}`}
        />
      </div>
    </div>
  );
};
