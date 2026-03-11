import { createSignal, For, Show } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { CommentForm } from './CommentForm';
import { useQuery, useDb, SyncedDb } from '@spooky-sync/client-solid';
import { useAuth } from '../lib/auth';
import { CommentBox } from './CommentBox';
import { ThreadSidebar } from './ThreadSidebar';
import { createHotkey, isInputActive } from '../lib/keyboard';
import SpookButton from './SpookButton';
import { schema } from '../schema.gen';
import { ProfilePicture } from './ProfilePicture';
import { Tooltip } from './Tooltip';

const createQuery = (
  db: SyncedDb<typeof schema>,
  {
    threadId,
    commentFilter,
    userId,
  }: {
    threadId: string;
    commentFilter: 'all' | 'mine';
    userId: string;
  }
) => {
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
  const db = useDb<typeof schema>();
  const auth = useAuth();
  const params = useParams();
  const navigate = useNavigate();
  const [commentFilter, setCommentFilter] = createSignal<'all' | 'mine'>('all');
  const [spookifySending, setSpookifySending] = createSignal(false);

  const threadResult = useQuery(() =>
    createQuery(db, {
      threadId: params.id,
      commentFilter: commentFilter(),
      userId: auth.user()?.id ?? '',
    })
  );
  const thread = () => threadResult.data() || null;

  // Query all threads for j/k navigation between threads
  const allThreadsResult = useQuery(() => {
    return db.query('thread').orderBy('title', 'asc').limit(20).build();
  });
  const allThreads = () => allThreadsResult.data() || [];

  const handleBack = () => {
    navigate('/');
  };

  const navigateToAdjacentThread = (direction: 1 | -1) => {
    const list = allThreads();
    if (list.length === 0) return;
    const currentIdx = list.findIndex((t) => t.id.split(':')[1] === params.id);
    if (currentIdx === -1) return;
    const nextIdx = currentIdx + direction;
    if (nextIdx < 0 || nextIdx >= list.length) return;
    navigate(`/thread/${list[nextIdx].id.split(':')[1]}`);
  };

  createHotkey('J', () => navigateToAdjacentThread(1));
  createHotkey('K', () => navigateToAdjacentThread(-1));
  createHotkey('R', () => {
    const textarea = document.querySelector('#comment-textarea') as HTMLTextAreaElement;
    textarea?.focus();
  });
  createHotkey('Escape', () => {
    if (isInputActive()) {
      (document.activeElement as HTMLElement).blur();
    } else {
      handleBack();
    }
  }, { ignoreInputs: false });

  const isAuthor = () => {
    const threadData = thread();
    const currentUser = auth.user();
    if (!threadData?.author?.id || !currentUser?.id) return false;
    return threadData.author.id === currentUser.id;
  };

  const handleTitleChange = async (newTitle: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update('thread', threadData.id, { title: newTitle }, { debounced: true });
  };

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
    await db.update('thread', threadData.id, { title: suggestion, title_suggestion: '' });
  };

  const handleDeclineTitle = async () => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update('thread', threadData.id, { title_suggestion: '' });
  };

  const handleAcceptContent = async (suggestion: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update('thread', threadData.id, { content: suggestion, content_suggestion: '' });
  };

  const handleDeclineContent = async () => {
    const threadData = thread();
    if (!threadData || !threadData.id || !isAuthor()) return;
    await db.update('thread', threadData.id, { content_suggestion: '' });
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
    <div class="fixed inset-0 top-14 z-40 bg-zinc-950">
      <div class="max-w-5xl mx-auto h-full flex">
        <ThreadSidebar
          activeThreadId={params.id}
          onNavigate={navigateToAdjacentThread}
          threads={allThreads()}
          isLoading={allThreadsResult.isLoading()}
        />

        <div class="flex-1 overflow-y-auto">
        <div class="max-w-3xl mx-auto w-full px-6 py-6">
        <Show
          when={thread()}
          fallback={
            <Show when={!threadResult.isLoading()} fallback={
              <div class="bg-surface/50 rounded-xl border border-white/[0.06] p-12 text-center">
                <p class="text-zinc-400 font-medium mb-1">Loading thread...</p>
              </div>
            }>
              <div class="bg-surface/50 rounded-xl border border-white/[0.06] p-12 text-center">
                <p class="text-zinc-400 font-medium mb-1">Thread not found</p>
                <p class="text-sm text-zinc-600">
                  This thread may have been deleted or doesn't exist.
                </p>
              </div>
            </Show>
          }
        >
          {(threadData) => (
            <div class="space-y-8">
              {/* ───── Thread Article ───── */}
              <article>
                {/* Author header */}
                <div class="flex items-center gap-3 mb-5">
                  <ProfilePicture
                    src={() => threadData().author?.profile_picture}
                    username={() => threadData().author?.username}
                    size="md"
                  />
                  <div>
                    <div class="text-sm font-medium text-zinc-200">
                      {threadData().author?.username || 'Unknown'}
                    </div>
                    <div class="text-xs text-zinc-600">
                      {new Date(threadData().created_at ?? 0).toLocaleDateString(undefined, {
                        month: 'long',
                        day: 'numeric',
                        year: 'numeric',
                        hour: '2-digit',
                        minute: '2-digit',
                      })}
                    </div>
                  </div>
                  <Show when={isAuthor()}>
                    <span class="ml-auto text-[11px] text-zinc-600 bg-surface border border-white/[0.06] rounded-full px-2.5 py-0.5">
                      Author
                    </span>
                  </Show>
                </div>

                {/* Title + Content card */}
                <div class="bg-surface/40 rounded-xl border border-white/[0.06] p-6">
                  <Show
                    when={isAuthor()}
                    fallback={
                      <>
                        <h1 class="text-2xl font-semibold mb-4 leading-tight">
                          {threadData().title || 'Untitled'}
                        </h1>
                        <div class="text-[15px] text-zinc-400 whitespace-pre-wrap leading-relaxed min-h-[80px]">
                          {threadData().content || 'No content yet...'}
                        </div>
                      </>
                    }
                  >
                    {/* Title Suggestion */}
                    <Show when={threadData().title_suggestion}>
                      <div class="mb-4 bg-zinc-800/50 border border-white/[0.06] rounded-lg p-4">
                        <div class="flex justify-between items-start mb-2">
                          <span class="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
                            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
                            </svg>
                            AI title suggestion
                          </span>
                          <div class="flex gap-2">
                            <button
                              onMouseDown={() => handleAcceptTitle(threadData().title_suggestion!)}
                              class="text-xs font-medium bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white px-3 py-1 rounded-md transition-colors duration-150"
                            >
                              Accept
                            </button>
                            <button
                              onMouseDown={() => handleDeclineTitle()}
                              class="text-xs font-medium text-zinc-500 hover:text-white px-3 py-1 transition-colors duration-150"
                            >
                              Dismiss
                            </button>
                          </div>
                        </div>
                        <div class="text-lg font-semibold text-white">{threadData().title_suggestion}</div>
                      </div>
                    </Show>

                    {/* Editable Title */}
                    <input
                      type="text"
                      value={threadData().title}
                      onInput={(e) => handleTitleChange(e.currentTarget.value)}
                      class="text-2xl font-semibold w-full bg-transparent border-none outline-none text-white placeholder-zinc-700 mb-4 leading-tight"
                      placeholder="Untitled"
                    />

                    {/* Content Suggestion */}
                    <Show when={threadData().content_suggestion}>
                      <div class="mb-4 bg-zinc-800/50 border border-white/[0.06] rounded-lg p-4">
                        <div class="flex justify-between items-start mb-2">
                          <span class="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
                            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
                            </svg>
                            AI content suggestion
                          </span>
                          <div class="flex gap-2">
                            <button
                              onMouseDown={() => handleAcceptContent(threadData().content_suggestion!)}
                              class="text-xs font-medium bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white px-3 py-1 rounded-md transition-colors duration-150"
                            >
                              Accept
                            </button>
                            <button
                              onMouseDown={() => handleDeclineContent()}
                              class="text-xs font-medium text-zinc-500 hover:text-white px-3 py-1 transition-colors duration-150"
                            >
                              Dismiss
                            </button>
                          </div>
                        </div>
                        <div class="text-sm text-zinc-300 whitespace-pre-wrap leading-relaxed">
                          {threadData().content_suggestion}
                        </div>
                      </div>
                    </Show>

                    {/* Editable Content */}
                    <textarea
                      value={threadData().content}
                      onInput={(e) => handleContentChange(e.currentTarget.value)}
                      class="w-full bg-transparent text-[15px] text-zinc-300 focus:text-white whitespace-pre-wrap outline-none resize-none min-h-[120px] leading-relaxed"
                      placeholder="Write something..."
                    />
                  </Show>
                </div>

                {/* Actions row beneath the card */}
                <div class="flex items-center justify-between mt-3 px-1">
                  <div class="flex items-center gap-4">
                    <Tooltip text="Reply" kbd="r">
                      <button
                        onMouseDown={() => {
                          const textarea = document.querySelector('#comment-textarea') as HTMLTextAreaElement;
                          textarea?.focus();
                        }}
                        class="inline-flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-300 transition-colors duration-150"
                      >
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M12 20.25c4.97 0 9-3.694 9-8.25s-4.03-8.25-9-8.25S3 7.444 3 12c0 2.104.859 4.023 2.273 5.48.432.447.74 1.04.586 1.641a4.483 4.483 0 01-.923 1.785A5.969 5.969 0 006 21c1.282 0 2.47-.402 3.445-1.087.81.22 1.668.337 2.555.337z" />
                        </svg>
                        {threadData().comments?.length || 0}
                      </button>
                    </Tooltip>
                  </div>

                  <SpookButton
                    loading={spookifySending() || spookifyJobLoading()}
                    loadingLabel="Processing..."
                    onClick={handleSpookify}
                  >
                    Spookify
                  </SpookButton>
                </div>
              </article>

              {/* ───── Replies Section ───── */}
              <div>
                <div class="flex items-center justify-between mb-4 pb-3 border-b border-white/[0.06]">
                  <h2 class="text-base font-semibold">
                    Replies
                    <span class="text-zinc-600 font-normal text-sm ml-1.5">
                      {threadData().comments?.length || 0}
                    </span>
                  </h2>

                  <Show when={auth.user()}>
                    <div class="flex text-xs bg-surface rounded-lg border border-white/[0.06] overflow-hidden">
                      <button
                        onMouseDown={() => setCommentFilter('all')}
                        class={`px-3 py-1.5 transition-colors duration-150 ${
                          commentFilter() === 'all'
                            ? 'bg-surface-hover text-white'
                            : 'text-zinc-500 hover:text-zinc-300'
                        }`}
                      >
                        All
                      </button>
                      <button
                        onMouseDown={() => setCommentFilter('mine')}
                        class={`px-3 py-1.5 transition-colors duration-150 ${
                          commentFilter() === 'mine'
                            ? 'bg-surface-hover text-white'
                            : 'text-zinc-500 hover:text-zinc-300'
                        }`}
                      >
                        Mine
                      </button>
                    </div>
                  </Show>
                </div>

                {/* Comment form */}
                <div class="mb-6">
                  <CommentForm thread={threadData} />
                </div>

                {/* Comments list */}
                <div class="space-y-1">
                  <For
                    each={threadData().comments ?? []}
                    fallback={
                      <div class="py-10 text-center text-sm text-zinc-600">
                        No replies yet. Be the first to respond.
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
      </div>
    </div>
  );
}
