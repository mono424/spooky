import React from 'react';

export const SchemaFilePreview: React.FC = () => {
  return (
    <div className="bg-surface/50 backdrop-blur-sm border border-surface-border rounded-xl p-6 lg:p-8">
      {/* File Header */}
      <div className="flex items-center gap-2 mb-4 text-sm text-text-tertiary">
        <svg
          className="w-4 h-4 text-accent-400"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
          />
        </svg>
        <span className="font-mono text-accent-400">schema.surql</span>
      </div>

      {/* Code Block */}
      <div className="font-mono text-xs lg:text-sm leading-relaxed space-y-2">
        {/* User Table */}
        <div>
          <span className="text-purple-400 font-bold">DEFINE TABLE</span>{' '}
          <span className="text-text-primary">user</span>{' '}
          <span className="text-purple-400 font-bold">SCHEMAFULL</span>
          <span className="text-gray-500">;</span>
        </div>
        <div>
          <span className="text-purple-400 font-bold">DEFINE FIELD</span>{' '}
          <span className="text-text-primary">username</span>{' '}
          <span className="text-purple-400 font-bold">ON</span>{' '}
          <span className="text-text-primary">user</span>
        </div>
        <div className="pl-4">
          <span className="text-purple-400 font-bold">TYPE</span>{' '}
          <span className="text-blue-400">string</span>
          <span className="text-gray-500">;</span>
        </div>
        <div>
          <span className="text-purple-400 font-bold">DEFINE FIELD</span>{' '}
          <span className="text-text-primary">email</span>{' '}
          <span className="text-purple-400 font-bold">ON</span>{' '}
          <span className="text-text-primary">user</span>
        </div>
        <div className="pl-4">
          <span className="text-purple-400 font-bold">TYPE</span>{' '}
          <span className="text-blue-400">string</span>
          <span className="text-gray-500">;</span>
        </div>

        {/* Spacing */}
        <div className="h-2"></div>

        {/* Thread Table */}
        <div>
          <span className="text-purple-400 font-bold">DEFINE TABLE</span>{' '}
          <span className="text-text-primary">thread</span>{' '}
          <span className="text-purple-400 font-bold">SCHEMAFULL</span>
          <span className="text-gray-500">;</span>
        </div>
        <div>
          <span className="text-purple-400 font-bold">DEFINE FIELD</span>{' '}
          <span className="text-text-primary">title</span>{' '}
          <span className="text-purple-400 font-bold">ON</span>{' '}
          <span className="text-text-primary">thread</span>
        </div>
        <div className="pl-4">
          <span className="text-purple-400 font-bold">TYPE</span>{' '}
          <span className="text-blue-400">string</span>
          <span className="text-gray-500">;</span>
        </div>
        <div>
          <span className="text-purple-400 font-bold">DEFINE FIELD</span>{' '}
          <span className="text-text-primary">author</span>{' '}
          <span className="text-purple-400 font-bold">ON</span>{' '}
          <span className="text-text-primary">thread</span>
        </div>
        <div className="pl-4">
          <span className="text-purple-400 font-bold">TYPE</span>{' '}
          <span className="text-accent-400">record&lt;user&gt;</span>
          <span className="text-gray-500">;</span>
        </div>
      </div>
    </div>
  );
};
