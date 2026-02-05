import { createSignal, For, Show } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { db } from '../db';
import { CommentForm } from './CommentForm';
import { useQuery } from '@spooky/client-solid';
import { useAuth } from '../lib/auth';
import { CommentBox } from './CommentBox';
import { ThreadSidebar } from './ThreadSidebar';
import { isInputActive, useKeyboard } from '../lib/keyboard';
import SpookButton from './SpookButton';

const createQuery = ({
  threadId,
  commentFilter,
  userId,
}: {
  threadId: string;
  commentFilter: 'all' | 'mine';
  userId: string;
}) => {
  return db
    .query('thread')
    .where({
      id: `thread:${threadId}`,
    })
    .related('author')
    .related('comments', (q) => {
      const withAuthor = q.related('author');
      if (commentFilter === 'mine' && userId) {
        return withAuthor.where({ author: userId });
      }
      return withAuthor.orderBy('created_at', 'desc').limit(10);
    })
    .related('jobs', (q) => {
      return q.where({ path: '/spookify' }).orderBy('created_at', 'desc').limit(1);
    })
    .one()
    .build();
};

export function ThreadDetail() {
  const auth = useAuth();
  const params = useParams();
  const navigate = useNavigate();
  const [commentFilter, setCommentFilter] = createSignal<'all' | 'mine'>('all');
  const [spookifySending, setSpookifySending] = createSignal(false);

  const threadResult = useQuery(db, () =>
    createQuery({
      threadId: params.id,
      commentFilter: commentFilter(),
      userId: auth.user()?.id ?? '',
    })
  );
  const thread = () => threadResult.data() || null;

  const handleBack = () => {
    navigate('/');
  };

  useKeyboard({
    r: (e) => {
      e.preventDefault();
      const textarea = document.querySelector('#comment-textarea') as HTMLTextAreaElement;
      textarea?.focus();
    },
    // We can use Escape here because we want it to work even if nothing is focused to go back
    Escape: () => {
      if (isInputActive()) {
        (document.activeElement as HTMLElement).blur();
      } else {
        handleBack();
      }
    },
  });

  // Check if current user is the author
  const isAuthor = () => {
    const threadData = thread();
    const currentUser = auth.user();
    if (!threadData?.author?.id || !currentUser?.id) return false;
    return threadData.author.id === currentUser.id;
  };

  // Auto-save title changes
  const handleTitleChange = async (newTitle: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update('thread', threadData.id, { title: newTitle }, { debounced: true });
  };

  // Auto-save content changes
  const handleContentChange = async (newContent: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update(
      'thread',
      threadData.id,
      { content: newContent },
      { debounced: { delay: 2000, key: 'recordId_x_fields' } }
    );
  };

  const handleAcceptTitle = async (suggestion: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;

    await db.update('thread', threadData.id, {
      title: suggestion,
      title_suggestion: '',
    });
  };

  const handleDeclineTitle = async () => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;

    await db.update('thread', threadData.id, {
      title_suggestion: '',
    });
  };

  const handleAcceptContent = async (suggestion: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;

    await db.update('thread', threadData.id, {
      content: suggestion,
      content_suggestion: '',
    });
  };

  const handleDeclineContent = async () => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;

    await db.update('thread', threadData.id, {
      content_suggestion: '',
    });
  };

  const handleSpookify = async () => {
    setSpookifySending(true);
    const threadData = thread();
    if (!threadData || !threadData.id) return;

    try {
      await db.run('api', '/spookify', { id: threadData.id }, { assignedTo: threadData.id });
    } catch (err) {
      console.error('Spookify failed:', err);
    }
    setSpookifySending(false);
  };

  const spookifyJobLoading = () =>
    ['pending', 'processing'].includes(thread()?.jobs?.[0]?.status ?? '');

  return (
    <div class="flex h-full">
      {/* Thread Sidebar */}
      <ThreadSidebar activeThreadId={params.id} />

      {/* Main Content */}
      <div class="flex-1 max-w-4xl mx-auto p-4 font-mono w-full">
        {/* Navigation Bar */}
        <div class="flex justify-between items-center mb-6 border-b border-gray-800 pb-2">
          <button
            onMouseDown={handleBack}
            class="text-xs uppercase font-bold text-gray-400 hover:text-white hover:underline decoration-white underline-offset-4 flex items-center gap-2 transition-none"
          >
            <span>&lt;&lt;</span> RETURN_TO_ROOT
          </button>

          <div class="flex items-center gap-6">
            <SpookButton
              loading={spookifySending() || spookifyJobLoading()}
              loadingLabel="HAUNTING..."
              onClick={handleSpookify}
            >
              SPOOKIFY
            </SpookButton>
            <div class="text-[10px] uppercase text-gray-600">
              MODE: {isAuthor() ? 'READ_WRITE' : 'READ_ONLY'}
            </div>
          </div>
        </div>

        <Show
          when={thread()}
          fallback={
            <div class="border-2 border-dashed border-red-900/50 p-12 text-center">
              <div class="text-red-500 font-bold uppercase tracking-widest mb-2">
                ! ERROR_404: FILE_NOT_FOUND
              </div>
              <div class="text-xs text-gray-500">
                The requested thread ID does not exist in the database.
              </div>
            </div>
          }
        >
          {(threadData) => (
            <div class="space-y-8">
              {/* Thread Content Wrapper */}
              <div
                class={`border-2 p-6 relative bg-black ${isAuthor() ? 'border-white' : 'border-gray-700'}`}
              >
                {/* Decorative Header */}
                <div
                  class={`absolute -top-3 left-4 bg-black px-2 text-xs font-bold uppercase border-x ${isAuthor() ? 'border-white' : 'border-gray-700'}`}
                >
                  {isAuthor() ? 'FILE_EDITOR' : 'FILE_VIEWER'}
                </div>

                <Show
                  when={isAuthor()}
                  fallback={
                    <>
                      {/* Read-Only Title */}
                      <div class="mb-6">
                        <label class="block text-[10px] text-gray-600 uppercase font-bold mb-1">
                          &gt; SUBJECT_LINE
                        </label>
                        <h1 class="text-2xl font-bold w-full text-gray-400 uppercase tracking-wide">
                          {threadData().title || 'UNTITLED_THREAD'}
                        </h1>
                      </div>

                      {/* Read-Only Content */}
                      <div class="mb-6">
                        <label class="block text-[10px] text-gray-600 uppercase font-bold mb-1">
                          &gt; DATA_CONTENT
                        </label>
                        <div class="w-full text-gray-400 whitespace-pre-wrap border-l-2 border-gray-700 pl-4 min-h-[150px] leading-relaxed">
                          {threadData().content || 'No content data...'}
                        </div>
                      </div>
                    </>
                  }
                >
                  {/* Title Suggestion */}
                  <Show when={threadData().title_suggestion}>
                    <div class="mb-6 border border-yellow-500/30 bg-yellow-900/10 p-4">
                      <div class="flex justify-between items-start mb-2">
                        <label class="text-[10px] text-yellow-500 uppercase font-bold tracking-wider">
                          &gt; AI_SUGGESTION_DETECTED
                        </label>
                        <div class="flex gap-2">
                          <button
                            onMouseDown={() => handleAcceptTitle(threadData().title_suggestion!)}
                            class="text-[10px] font-bold bg-yellow-500 text-black px-3 py-1 uppercase hover:bg-yellow-400 transition-colors"
                          >
                            [ ACCEPT ]
                          </button>
                          <button
                            onMouseDown={() => handleDeclineTitle()}
                            class="text-[10px] font-bold text-yellow-600 px-3 py-1 uppercase hover:text-yellow-400 transition-colors"
                          >
                            [ DECLINE ]
                          </button>
                        </div>
                      </div>
                      <div class="text-xl font-bold text-yellow-100 uppercase tracking-wide">
                        {threadData().title_suggestion}
                      </div>
                    </div>
                  </Show>

                  {/* Editable Title */}
                  <div class="mb-6 group">
                    <label class="block text-[10px] text-gray-500 uppercase font-bold mb-1 group-focus-within:text-white">
                      &gt; SUBJECT_LINE
                    </label>
                    <input
                      type="text"
                      value={threadData().title}
                      onInput={(e) => handleTitleChange(e.currentTarget.value)}
                      class="text-2xl font-bold w-full bg-black border-b-2 border-transparent focus:border-white outline-none text-white placeholder-gray-700 uppercase tracking-wide transition-none rounded-none"
                      placeholder="UNTITLED_THREAD"
                    />
                  </div>

                  {/* Content Suggestion */}
                  <Show when={threadData().content_suggestion}>
                    <div class="mb-6 border border-yellow-500/30 bg-yellow-900/10 p-4">
                      <div class="flex justify-between items-start mb-2">
                        <label class="text-[10px] text-yellow-500 uppercase font-bold tracking-wider">
                          &gt; AI_CONTENT_OPTIMIZATION
                        </label>
                        <div class="flex gap-2">
                          <button
                            onMouseDown={() =>
                              handleAcceptContent(threadData().content_suggestion!)
                            }
                            class="text-[10px] font-bold bg-yellow-500 text-black px-3 py-1 uppercase hover:bg-yellow-400 transition-colors"
                          >
                            [ ACCEPT ]
                          </button>
                          <button
                            onMouseDown={() => handleDeclineContent()}
                            class="text-[10px] font-bold text-yellow-600 px-3 py-1 uppercase hover:text-yellow-400 transition-colors"
                          >
                            [ DECLINE ]
                          </button>
                        </div>
                      </div>
                      <div class="text-sm text-yellow-100 whitespace-pre-wrap leading-relaxed border-l-2 border-yellow-500/30 pl-4">
                        {threadData().content_suggestion}
                      </div>
                    </div>
                  </Show>

                  {/* Editable Content */}
                  <div class="mb-6 group">
                    <label class="block text-[10px] text-gray-500 uppercase font-bold mb-1 group-focus-within:text-white">
                      &gt; DATA_CONTENT
                    </label>
                    <textarea
                      value={threadData().content}
                      onInput={(e) => handleContentChange(e.currentTarget.value)}
                      class="w-full bg-black text-gray-300 focus:text-white whitespace-pre-wrap border-l-2 border-gray-800 focus:border-white outline-none pl-4 resize-none min-h-[150px] leading-relaxed transition-none rounded-none"
                      placeholder="No content data..."
                    />
                  </div>
                </Show>

                {/* Metadata Footer */}
                <div class="flex justify-between items-center text-[10px] uppercase text-gray-500 border-t border-dashed border-gray-700 pt-3 font-bold tracking-wider">
                  <div class="flex gap-4">
                    <span>
                      AUTHOR:{' '}
                      <span class="text-white">{threadData().author?.username || 'UNKNOWN'}</span>
                    </span>
                    <span>ID: {threadData().id?.slice(0, 8)}</span>
                  </div>
                  <span>DATE: {new Date(threadData().created_at ?? 0).toLocaleDateString()}</span>
                </div>
              </div>

              {/* Comments Section */}
              <div class="space-y-6">
                <div class="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4 border-b-2 border-white pb-2">
                  <h2 class="text-xl font-bold uppercase tracking-widest flex items-center gap-2">
                    <span>//</span> ATTACHED_LOGS{' '}
                    <span class="text-xs align-top">({threadData().comments?.length || 0})</span>
                  </h2>

                  <Show when={auth.user()}>
                    <div class="flex text-xs font-bold">
                      <button
                        onMouseDown={() => setCommentFilter('all')}
                        class={`px-3 py-1 border-2 border-r-0 border-white uppercase transition-none ${
                          commentFilter() === 'all'
                            ? 'bg-white text-black'
                            : 'bg-black text-white hover:bg-gray-900'
                        }`}
                      >
                        [ ALL_LOGS ]
                      </button>
                      <button
                        onMouseDown={() => setCommentFilter('mine')}
                        class={`px-3 py-1 border-2 border-white uppercase transition-none ${
                          commentFilter() === 'mine'
                            ? 'bg-white text-black'
                            : 'bg-black text-white hover:bg-gray-900'
                        }`}
                      >
                        [ MY_LOGS ]
                      </button>
                    </div>
                  </Show>
                </div>

                {/* Comment Form */}
                <div class="bg-black border border-gray-800 p-4 hover:border-white transition-colors">
                  <div class="text-[10px] uppercase text-gray-500 mb-2 font-bold">
                    &gt; APPEND_NEW_ENTRY
                  </div>
                  <CommentForm thread={threadData} />
                </div>

                {/* Comments List */}
                <div class="space-y-4 pl-4 border-l border-dashed border-gray-800">
                  <For
                    each={threadData().comments ?? []}
                    fallback={
                      <div class="text-left py-4 text-gray-600 text-xs font-mono uppercase">
                        &gt; NULL_DATA: No logs found. Be the first to append.
                      </div>
                    }
                  >
                    {(comment) => <CommentBox comment={comment} />}
                  </For>
                </div>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
