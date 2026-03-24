import React from 'react';

export const TerminalCommand: React.FC = () => {
  return (
    <div className="border border-surface-border bg-surface shadow-xl rounded-xl overflow-hidden font-mono text-sm">
      {/* Window Chrome */}
      <div className="flex justify-between items-center bg-surface-elevated border-b border-surface-border px-4 py-3 text-xs select-none">
        <div className="flex items-center gap-3">
          <div className="flex gap-1.5">
            <div className="w-3 h-3 rounded-full bg-red-500/50"></div>
            <div className="w-3 h-3 rounded-full bg-yellow-500/50"></div>
            <div className="w-3 h-3 rounded-full bg-accent-500/50"></div>
          </div>
          <span className="text-text-tertiary">terminal</span>
        </div>
      </div>

      {/* Terminal Body */}
      <div className="bg-[#0a0a0a] p-6 space-y-3 min-h-[300px]">
        {/* Command */}
        <div className="flex items-start gap-2">
          <span className="text-green-400">$</span>
          <span className="text-green-400">sp00ky generate --target ts,dart,zod</span>
        </div>

        {/* Success Messages */}
        <div className="space-y-1 pl-4">
          <div className="flex items-center gap-2">
            <svg
              className="w-3 h-3 text-accent-500 flex-shrink-0"
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
            <span className="text-accent-400 text-xs">Generated schema.gen.ts</span>
          </div>
          <div className="flex items-center gap-2">
            <svg
              className="w-3 h-3 text-accent-500 flex-shrink-0"
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
            <span className="text-accent-400 text-xs">Generated schema.g.dart</span>
          </div>
          <div className="flex items-center gap-2">
            <svg
              className="w-3 h-3 text-accent-500 flex-shrink-0"
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
            <span className="text-accent-400 text-xs">Generated schema.zod.ts</span>
          </div>
        </div>

        {/* Schema Preview - matches SchemaFilePreview */}
        <div className="mt-4 pt-4 border-t border-gray-800">
          <div className="text-text-tertiary text-xs mb-2">schema.surql:</div>
          <div className="text-xs leading-relaxed space-y-1 text-text-tertiary/80">
            <div>
              <span className="text-purple-400">DEFINE TABLE</span>{' '}
              <span className="text-white">user</span>{' '}
              <span className="text-purple-400">SCHEMAFULL</span>
              <span className="text-gray-500">;</span>
            </div>
            <div>
              <span className="text-purple-400">DEFINE FIELD</span>{' '}
              <span className="text-white">username</span>{' '}
              <span className="text-purple-400">ON</span>{' '}
              <span className="text-white">user</span>
            </div>
            <div className="pl-4">
              <span className="text-purple-400">TYPE</span>{' '}
              <span className="text-blue-400">string</span>
              <span className="text-gray-500">;</span>
            </div>
            <div>
              <span className="text-purple-400">DEFINE FIELD</span>{' '}
              <span className="text-white">email</span>{' '}
              <span className="text-purple-400">ON</span>{' '}
              <span className="text-white">user</span>
            </div>
            <div className="pl-4">
              <span className="text-purple-400">TYPE</span>{' '}
              <span className="text-blue-400">string</span>
              <span className="text-gray-500">;</span>
            </div>
            <div className="h-1"></div>
            <div>
              <span className="text-purple-400">DEFINE TABLE</span>{' '}
              <span className="text-white">thread</span>{' '}
              <span className="text-purple-400">SCHEMAFULL</span>
              <span className="text-gray-500">;</span>
            </div>
            <div>
              <span className="text-purple-400">DEFINE FIELD</span>{' '}
              <span className="text-white">title</span>{' '}
              <span className="text-purple-400">ON</span>{' '}
              <span className="text-white">thread</span>
            </div>
            <div className="pl-4">
              <span className="text-purple-400">TYPE</span>{' '}
              <span className="text-blue-400">string</span>
              <span className="text-gray-500">;</span>
            </div>
            <div>
              <span className="text-purple-400">DEFINE FIELD</span>{' '}
              <span className="text-white">author</span>{' '}
              <span className="text-purple-400">ON</span>{' '}
              <span className="text-white">thread</span>
            </div>
            <div className="pl-4">
              <span className="text-purple-400">TYPE</span>{' '}
              <span className="text-accent-400">record&lt;user&gt;</span>
              <span className="text-gray-500">;</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
