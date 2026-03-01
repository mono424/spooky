import { For } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useQuery, useDb } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

interface ThreadSidebarProps {
  activeThreadId?: string;
}

export function ThreadSidebar(props: ThreadSidebarProps) {
  const db = useDb<typeof schema>();
  const navigate = useNavigate();

  const threadsResult = useQuery(() => {
    return db.query('thread').orderBy('title', 'asc').limit(20).build();
  });

  const threads = () => threadsResult.data() || [];

  const handleThreadClick = (threadId: string) => {
    const id = threadId.split(':')[1];
    navigate(`/thread/${id}`);
  };

  const isActive = (threadId: string) => {
    const id = threadId.split(':')[1];
    return id === props.activeThreadId;
  };

  return (
    <aside class="w-60 flex-shrink-0 hidden md:block h-full border-r border-white/[0.06]">
      <div class="flex flex-col h-full">
        {/* Header */}
        <div class="px-4 py-3 border-b border-white/[0.06]">
          <h3 class="text-xs font-medium text-zinc-500 tracking-wide">Threads</h3>
        </div>

        {/* Thread list */}
        <nav class="flex-1 min-h-0 overflow-y-auto py-2">
          <For
            each={threads()}
            fallback={
              <div class="px-4 py-8 text-center">
                <div class="text-xs text-zinc-600">No threads</div>
              </div>
            }
          >
            {(thread) => (
              <button
                onMouseDown={() => handleThreadClick(thread.id)}
                class={`w-full text-left px-4 py-2.5 text-sm transition-colors duration-150 border-l-2 ${
                  isActive(thread.id)
                    ? 'border-accent text-white bg-accent/5'
                    : 'border-transparent text-zinc-500 hover:text-zinc-300 hover:bg-surface/50'
                }`}
              >
                <div class="truncate">
                  {thread.title || 'Untitled'}
                </div>
              </button>
            )}
          </For>
        </nav>

        {/* Footer — back link */}
        <div class="px-4 py-3 border-t border-white/[0.06]">
          <button
            onMouseDown={() => navigate('/')}
            class="inline-flex items-center gap-1.5 text-xs text-zinc-600 hover:text-zinc-300 transition-colors duration-150 w-full"
          >
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
            </svg>
            Back to feed
          </button>
        </div>
      </div>
    </aside>
  );
}
