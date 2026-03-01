import React from 'react';
import { LiveCodeEditor } from './LiveCodeEditor';

const solidQueryCode = `import { useQuery } from "@spooky-sync/client-solid";
import { db } from "../db";

const ThreadList = () => {
  const result = useQuery(db, () =>
    db.query("thread")
      .related("comments")
      .orderBy("created_at", "desc")
      .limit(10)
      .build()
  );

  return (
    <For each={result.data()}>
      {(thread) => (
        <div>{thread.title} by {thread.author.username}</div>
      )}
    </For>
  );
};`;

export const DXPane1: React.FC = () => {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-16 items-start">
      {/* Left: LiveCodeEditor */}
      <div className="order-2 lg:order-1">
        <LiveCodeEditor filename="ThreadList.tsx" initialCode={solidQueryCode} />
      </div>

      {/* Right: Feature cards with ALL information */}
      <div className="order-1 lg:order-2 space-y-6">
        {/* Context-Aware Autocomplete card */}
        <div>
          <div className="border border-surface-border bg-surface rounded-xl p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 bg-brand-500/10 rounded-lg flex items-center justify-center text-brand-500">
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z" />
                  <path d="M5 3v4" />
                  <path d="M9 3v4" />
                  <path d="M3 5h4" />
                  <path d="M3 9h4" />
                </svg>
              </div>
              <div>
                <div className="text-lg font-semibold text-text-primary">
                  Context-Aware Autocomplete
                </div>
              </div>
            </div>

            <p className="text-body-sm text-text-tertiary leading-relaxed">
              Context-aware autocomplete knows your schema inside out. It suggests sort fields,
              relations, and filter operations appropriate for your current query context.
            </p>
          </div>
        </div>

        {/* Real-Time Feedback card */}
        <div>
          <div className="border border-surface-border bg-surface rounded-xl p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 bg-accent-500/10 rounded-lg flex items-center justify-center text-accent-500">
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-1 1.5-2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5" />
                  <path d="M9 18h6" />
                  <path d="M10 22h4" />
                </svg>
              </div>
              <div>
                <div className="text-lg font-semibold text-text-primary">Real-Time Feedback</div>
              </div>
            </div>

            <p className="text-body-sm text-text-tertiary leading-relaxed">
              Your queries are continuously validated against your schema. Type mismatches,
              invalid fields, and logic errors are detected instantly and highlighted directly
              inside the code editor, ensuring your code is correct before execution. Try
              yourself and fix the type error on the left.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};
