import { Accessor, createSignal } from 'solid-js';
import { useAuth } from '../lib/auth';
import { RecordId, Uuid, useDb } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

interface CommentFormProps {
  thread: Accessor<{ id: string }>;
  onCommentAdded?: () => void;
}

export function CommentForm(props: CommentFormProps) {
  const db = useDb<typeof schema>();
  const auth = useAuth();
  const [content, setContent] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);
  const [isFocused, setIsFocused] = createSignal(false);

  const userInitial = () => {
    const name = auth.user()?.username;
    return name ? name.charAt(0).toUpperCase() : '?';
  };

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!content().trim() || isLoading()) return;

    setIsLoading(true);
    try {
      const user = auth.user();
      if (!user) {
        throw new Error('You must be logged in to post a comment');
      }

      const commentId = new RecordId('comment', Uuid.v4().toString().replace(/-/g, ''));

      await db.create(commentId.toString(), {
        thread: props.thread().id,
        content: content().trim(),
        author: user.id,
      });

      console.log('[CommentForm] Comment created with ID:', commentId.toString());

      setContent('');
      props.onCommentAdded?.();
    } catch (error) {
      console.error('Failed to create comment:', error);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} class="w-full">
      <div class="flex gap-3">
        {/* User avatar */}
        <div class="w-8 h-8 rounded-full bg-accent/15 text-accent flex items-center justify-center text-xs font-semibold flex-shrink-0 mt-1">
          {userInitial()}
        </div>

        <div class="flex-1">
          <textarea
            id="comment-textarea"
            value={content()}
            onInput={(e) => setContent(e.currentTarget.value)}
            onFocus={() => setIsFocused(true)}
            onBlur={() => setIsFocused(false)}
            placeholder="Write a reply..."
            rows={isFocused() || content().length > 0 ? '3' : '1'}
            class="w-full bg-surface text-white px-4 py-2.5 border border-white/[0.06] focus:border-accent/40 outline-none resize-none rounded-xl placeholder-zinc-600 text-sm leading-relaxed block transition-all duration-150"
            required
          />

          {(isFocused() || content().length > 0) && (
            <div class="flex justify-between items-center mt-2">
              <span class="text-xs text-zinc-600">
                {content().length > 0 ? `${content().length} characters` : ''}
              </span>

              <button
                type="submit"
                disabled={isLoading() || !content().trim()}
                class="bg-accent hover:bg-accent-hover text-white px-4 py-1.5 rounded-lg text-sm font-medium transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isLoading() ? 'Posting...' : 'Reply'}
              </button>
            </div>
          )}
        </div>
      </div>
    </form>
  );
}
