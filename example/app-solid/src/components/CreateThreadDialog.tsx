import { createSignal, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useAuth } from '../lib/auth';
import { Uuid, useDb } from '@spooky-sync/client-solid';
import { RecordId } from 'surrealdb';
import { schema } from '../schema.gen';

interface CreateThreadDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CreateThreadDialog(props: CreateThreadDialogProps) {
  const db = useDb<typeof schema>();
  const navigate = useNavigate();
  const auth = useAuth();
  const [title, setTitle] = createSignal('');
  const [content, setContent] = createSignal('');
  const [error, setError] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!title().trim() || !content().trim() || isLoading()) return;

    setError('');
    setIsLoading(true);

    try {
      const user = auth.user();
      if (!user) {
        throw new Error('You must be logged in to create a thread');
      }

      const genId = Uuid.v4().toString().replace(/-/g, '');
      const threadId = `thread:${genId}`;
      await db.create(threadId, {
        title: title().trim(),
        content: content().trim(),
        author: new RecordId('user', user.id.toString().split(':')[1]),
        active: true,
      });

      handleClose();
      navigate(`/thread/${genId}`);
    } catch (err) {
      console.error('Failed to create thread:', err);
      setError(err instanceof Error ? err.message : 'Failed to create thread');
    } finally {
      setIsLoading(false);
    }
  };

  const handleClose = () => {
    setTitle('');
    setContent('');
    setError('');
    props.onClose();
  };

  return (
    <Show when={props.isOpen}>
      <div class="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-[100] p-4" onMouseDown={handleClose}>
        <div
          class="animate-slide-up bg-surface border border-white/[0.06] rounded-xl w-full max-w-2xl shadow-2xl max-h-[90vh] flex flex-col"
          onMouseDown={(e) => e.stopPropagation()}
        >
          {/* Header */}
          <div class="flex justify-between items-center px-6 pt-6 pb-2 flex-shrink-0">
            <h2 class="text-lg font-semibold">New thread</h2>
            <button
              onMouseDown={handleClose}
              class="text-zinc-500 hover:text-white transition-colors duration-150 p-1"
              aria-label="Close"
            >
              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          {/* Content */}
          <div class="px-6 pb-6 pt-4 overflow-y-auto">
            <form onSubmit={handleSubmit} class="space-y-4">
              <div>
                <div class="flex justify-between items-end mb-1.5">
                  <label for="title" class="text-sm font-medium text-zinc-400">
                    Title
                  </label>
                  <span class="text-xs text-zinc-600">
                    {title().length}/200
                  </span>
                </div>
                <input
                  id="title"
                  type="text"
                  value={title()}
                  onInput={(e) => setTitle(e.currentTarget.value)}
                  required
                  maxlength="200"
                  class="w-full bg-zinc-950 border border-white/[0.06] rounded-lg px-4 py-2.5 text-white focus:outline-none focus:border-accent/50 transition-colors duration-150 placeholder-zinc-600 text-sm"
                  placeholder="Enter a title"
                  autocomplete="off"
                />
              </div>

              <div>
                <label for="content" class="block text-sm font-medium text-zinc-400 mb-1.5">
                  Content
                </label>
                <textarea
                  id="content"
                  value={content()}
                  onInput={(e) => setContent(e.currentTarget.value)}
                  required
                  rows="10"
                  class="w-full bg-zinc-950 border border-white/[0.06] rounded-lg p-4 text-white focus:outline-none focus:border-accent/50 transition-colors duration-150 placeholder-zinc-600 text-sm resize-none leading-relaxed block"
                  placeholder="What's on your mind?"
                />
              </div>

              <Show when={error()}>
                <div class="bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 p-3 text-sm">
                  {error()}
                </div>
              </Show>

              <div class="flex justify-end gap-3 pt-2">
                <button
                  type="button"
                  onMouseDown={handleClose}
                  class="px-5 py-2.5 text-sm text-zinc-400 hover:text-white transition-colors duration-150"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={isLoading() || !title().trim() || !content().trim()}
                  class="bg-accent hover:bg-accent-hover text-white px-6 py-2.5 rounded-lg font-medium transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed text-sm"
                >
                  {isLoading() ? 'Publishing...' : 'Publish'}
                </button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </Show>
  );
}
