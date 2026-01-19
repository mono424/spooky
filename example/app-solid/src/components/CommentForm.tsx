import { Accessor, createSignal } from 'solid-js';
import { db } from '../db';
import { useAuth } from '../lib/auth';
import { RecordId, Uuid } from '@spooky/client-solid';

interface CommentFormProps {
  thread: Accessor<{ id: string }>;
  onCommentAdded?: () => void;
}

export function CommentForm(props: CommentFormProps) {
  const auth = useAuth();
  const [content, setContent] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!content().trim() || isLoading()) return;

    setIsLoading(true);
    try {
      const user = auth.user();
      if (!user) {
        throw new Error('You must be logged in to post a comment');
      }

      // Generate a record ID before creating
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
    <form onSubmit={handleSubmit} class="w-full font-mono group">
      <div class="relative">
        {/* Terminal Prompt Indicator */}
        <div class="absolute top-3 left-3 text-gray-500 select-none group-focus-within:text-white transition-none font-bold">
          &gt;
        </div>

        <textarea
          id="comment-textarea"
          value={content()}
          onInput={(e) => setContent(e.currentTarget.value)}
          placeholder="INPUT_COMMENT_DATA..."
          rows="3"
          class="w-full bg-black text-white pl-8 pr-4 py-3 border-2 border-gray-800 focus:border-white outline-none resize-none rounded-none placeholder-gray-800 text-sm leading-relaxed block transition-none"
          required
        />

        {/* Decorative corner accent */}
        <div class="absolute bottom-0 right-0 w-3 h-3 border-b-2 border-r-2 border-gray-800 group-focus-within:border-white pointer-events-none transition-none"></div>
      </div>

      <div class="flex justify-between items-center mt-3">
        {/* Character count / status */}
        <div class="text-[10px] text-gray-600 uppercase tracking-widest">
          {content().length > 0 ? (
            <span class="text-white">BUFFER: {content().length} CHARS</span>
          ) : (
            <span>STATUS: IDLE</span>
          )}
        </div>

        <button
          type="submit"
          disabled={isLoading() || !content().trim()}
          class="bg-black text-white border-2 border-white px-6 py-2 uppercase font-bold text-xs hover:bg-white hover:text-black transition-none disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-black disabled:hover:text-white tracking-wider"
        >
          {isLoading() ? <span class="animate-pulse">TRANSMITTING...</span> : '[ EXECUTE_POST ]'}
        </button>
      </div>
    </form>
  );
}
