import { For, createSignal, createEffect } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useQuery, useDb } from '@spooky-sync/client-solid';
import { useKeyboard } from '../lib/keyboard';
import { schema } from '../schema.gen';
import { ProfilePicture } from './ProfilePicture';

export function ThreadList() {
  const db = useDb<typeof schema>();
  const navigate = useNavigate();

  const [sort, setSort] = createSignal('desc');
  const [selectedIndex, setSelectedIndex] = createSignal(-1);

  const threadsResult = useQuery(() => {
    let q = db.query('thread').related('author');
    q = q.orderBy('title', sort() as 'asc' | 'desc');
    return q.limit(10).build();
  });

  const threads = () => threadsResult.data() || [];

  // Reset selection when threads change
  createEffect(() => {
    threads();
    setSelectedIndex(-1);
  });

  const handleThreadClick = (threadId: string) => {
    navigate(`/thread/${threadId}`);
  };

  // Scroll the selected thread card into view
  const scrollSelectedIntoView = (index: number) => {
    const el = document.querySelector(`[data-thread-index="${index}"]`);
    el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  };

  useKeyboard({
    j: () => {
      const len = threads().length;
      if (len === 0) return;
      const next = Math.min(selectedIndex() + 1, len - 1);
      setSelectedIndex(next);
      scrollSelectedIntoView(next);
    },
    k: () => {
      const len = threads().length;
      if (len === 0) return;
      const prev = Math.max(selectedIndex() - 1, 0);
      setSelectedIndex(prev);
      scrollSelectedIntoView(prev);
    },
    Enter: () => {
      const idx = selectedIndex();
      const list = threads();
      if (idx >= 0 && idx < list.length) {
        handleThreadClick(list[idx].id.split(':')[1]);
      }
    },
  });

  return (
    <div class="w-full">
      {/* Action Bar */}
      <div class="flex justify-between items-center mb-6 pt-2">
        <h1 class="text-xl font-semibold tracking-tight">Feed</h1>

        <div class="flex items-center gap-3">
          <select
            value={sort()}
            onInput={(e) => setSort(e.currentTarget.value)}
            class="bg-surface text-zinc-400 border border-white/[0.06] text-sm px-3 py-1.5 rounded-lg outline-none focus:border-accent/50 cursor-pointer transition-colors duration-150"
          >
            <option value="desc">Newest</option>
            <option value="asc">Oldest</option>
          </select>

          <button
            onMouseDown={() => navigate('/create-thread')}
            class="inline-flex items-center gap-2 bg-accent hover:bg-accent-hover text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors duration-150"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
            </svg>
            New thread
          </button>
        </div>
      </div>

      {/* Thread List */}
      <div class="space-y-3">
        <For
          each={threads()}
          fallback={
            <div class="bg-surface/50 rounded-xl border border-white/[0.06] py-16 text-center">
              <p class="text-zinc-500 text-sm">No threads yet</p>
              <p class="text-zinc-600 text-xs mt-1">Create the first one to get started.</p>
            </div>
          }
        >
          {(thread, index) => (
            <div
              data-thread-index={index()}
              onMouseDown={() => handleThreadClick(thread.id.split(':')[1])}
              onMouseEnter={() => setSelectedIndex(index())}
              class={`border rounded-xl p-5 cursor-pointer transition-colors duration-150 group ${
                selectedIndex() === index()
                  ? 'bg-surface border-accent/40 ring-1 ring-accent/20'
                  : 'bg-surface/40 hover:bg-surface border-white/[0.06]'
              }`}
            >
              <div class="flex items-start gap-4">
                {/* Avatar */}
                <ProfilePicture
                  src={() => thread.author?.profile_picture}
                  username={() => thread.author?.username}
                  size="md"
                />

                <div class="flex-1 min-w-0">
                  {/* Author + time */}
                  <div class="flex items-center gap-2 mb-1">
                    <span class="text-sm font-medium text-zinc-300">
                      {thread.author?.username || 'Anonymous'}
                    </span>
                    <span class="text-zinc-700">&middot;</span>
                    <span class="text-xs text-zinc-600">
                      {new Date(thread.created_at ?? 0).toLocaleDateString(undefined, {
                        month: 'short',
                        day: '2-digit',
                        hour: '2-digit',
                        minute: '2-digit',
                      })}
                    </span>
                  </div>

                  {/* Title */}
                  <h2 class="text-[15px] font-semibold text-white group-hover:text-accent transition-colors duration-150 mb-1.5">
                    {thread.title}
                  </h2>

                  {/* Content preview */}
                  <p class="text-sm text-zinc-500 line-clamp-2 leading-relaxed">
                    {thread.content}
                  </p>
                </div>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
