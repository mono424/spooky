import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { CommentForm } from './CommentForm';
import type { SyncedDb } from '@spooky-sync/client-solid';
import { RecordId, useQuery, useDb, useCrdtField } from '@spooky-sync/client-solid';
import { useAuth } from '../lib/auth';
import { CommentBox } from './CommentBox';
import { ThreadSidebar } from './ThreadSidebar';
import { createHotkey, isInputActive } from '../lib/keyboard';
import { SpookButton } from './SpookButton';
import type { schema } from '../schema.gen';
import { ProfilePicture } from './ProfilePicture';
import { Tooltip } from './Tooltip';
import { CollaborativeEditor } from './CollaborativeEditor';
import { ShareDialog } from './ShareDialog';
import { MoreHorizontal } from 'lucide-solid';

interface CollaboratorRow {
  relationId: string;
  user: { id: string; username?: string; profile_picture?: string | null };
}

const parseRecordId = (id: string): RecordId => {
  const idx = id.indexOf(':');
  if (idx <= 0) throw new Error(`Invalid record id: ${id}`);
  return new RecordId(id.slice(0, idx), id.slice(idx + 1));
};

const stringifyId = (v: any): string =>
  typeof v === 'string' ? v : v instanceof RecordId ? v.toString() : v ? String(v) : '';

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
  createHotkey(
    'Escape',
    () => {
      if (isInputActive()) {
        (document.activeElement as HTMLElement).blur();
      } else {
        handleBack();
      }
    },
    { ignoreInputs: false }
  );

  const isAuthor = () => {
    const threadData = thread();
    const currentUser = auth.user();
    if (!threadData?.author?.id || !currentUser?.id) return false;
    return threadData.author.id === currentUser.id;
  };

  const [collaborators, setCollaborators] = createSignal<CollaboratorRow[]>([]);
  const [shareOpen, setShareOpen] = createSignal(false);
  const [menuOpen, setMenuOpen] = createSignal(false);

  const handleMenuClickOutside = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (!target.closest('[data-thread-menu]')) setMenuOpen(false);
  };
  document.addEventListener('mousedown', handleMenuClickOutside);
  onCleanup(() => document.removeEventListener('mousedown', handleMenuClickOutside));

  const handleDelete = async () => {
    const threadData = thread();
    if (!threadData?.id || !isAuthor()) return;
    if (!confirm('Delete this post? This cannot be undone.')) return;
    try {
      await db.delete('thread', threadData.id);
      navigate('/');
    } catch (e) {
      console.error('[ThreadDetail] failed to delete thread', e);
    }
  };

  const refreshCollaborators = async (threadId: string) => {
    try {
      const result = await db.useRemote(async (s) =>
        s.query<[Array<{ id: any; user: any }>]>(
          'SELECT id, in.* AS user FROM collaborates_on WHERE out = $t',
          { t: parseRecordId(threadId) }
        )
      );
      const rows = result?.[0] ?? [];
      setCollaborators(
        rows
          .filter((r) => r.user)
          .map((r) => ({
            relationId: stringifyId(r.id),
            user: { ...r.user, id: stringifyId(r.user.id) },
          }))
      );
    } catch (e) {
      console.error('[ThreadDetail] failed to load collaborators', e);
    }
  };

  createEffect(() => {
    const t = thread()?.id;
    if (t) refreshCollaborators(t);
  });

  const canEdit = () => {
    if (isAuthor()) return true;
    const me = auth.user()?.id;
    if (!me) return false;
    return collaborators().some((c) => c.user.id === me);
  };

  const removeCollaborator = async (relationId: string) => {
    try {
      await db.useRemote(async (s) => s.delete(parseRecordId(relationId)));
      const t = thread()?.id;
      if (t) await refreshCollaborators(t);
    } catch (e) {
      console.error('[ThreadDetail] failed to remove collaborator', e);
    }
  };

  // CRDT fields for collaborative editing
  const titleCrdtField = useCrdtField(
    'thread',
    () => thread()?.id ? `thread:${params.id}` : undefined,
    'title',
    () => thread()?.title
  );
  const contentCrdtField = useCrdtField(
    'thread',
    () => thread()?.id ? `thread:${params.id}` : undefined,
    'content',
    () => thread()?.content
  );

  const handleTitleChange = async (newTitle: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !canEdit()) return;
    await db.update('thread', threadData.id, { title: newTitle }, { debounced: true });
  };

  const handleContentChange = async (newContent: string) => {
    const threadData = thread();
    if (!threadData || !threadData.id || !canEdit()) return;
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
                <Show
                  when={!threadResult.isLoading()}
                  fallback={
                    <div class="bg-surface/50 rounded-xl border border-white/[0.06] p-12 text-center">
                      <p class="text-zinc-400 font-medium mb-1">Loading thread...</p>
                    </div>
                  }
                >
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

                      <div
                        class={`flex items-center gap-2 ${isAuthor() ? '' : 'ml-auto'}`}
                      >
                        <Show when={collaborators().length > 0}>
                          <div class="flex -space-x-2">
                            <For each={collaborators()}>
                              {(c) => (
                                <div class="relative group">
                                  <Tooltip text={c.user.username || 'Collaborator'}>
                                    <div class="ring-2 ring-zinc-950 rounded-full">
                                      <ProfilePicture
                                        src={() => c.user.profile_picture}
                                        username={() => c.user.username}
                                        size="sm"
                                      />
                                    </div>
                                  </Tooltip>
                                  <Show when={isAuthor()}>
                                    <button
                                      onMouseDown={() => removeCollaborator(c.relationId)}
                                      class="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-zinc-900 border border-white/[0.06] text-zinc-500 hover:text-red-400 text-[10px] leading-none opacity-0 group-hover:opacity-100 transition-opacity"
                                      title="Remove collaborator"
                                      aria-label="Remove collaborator"
                                    >
                                      ×
                                    </button>
                                  </Show>
                                </div>
                              )}
                            </For>
                          </div>
                        </Show>
                        <Show when={isAuthor()}>
                          <button
                            onMouseDown={() => setShareOpen(true)}
                            class="text-xs font-medium bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white px-3 py-1 rounded-md transition-colors duration-150"
                          >
                            Share
                          </button>

                          <div class="relative" data-thread-menu>
                            <button
                              onMouseDown={() => setMenuOpen(!menuOpen())}
                              class="inline-flex items-center justify-center w-8 h-8 text-zinc-500 hover:text-white rounded-lg transition-colors duration-150"
                              title="More"
                              aria-label="More options"
                            >
                              <MoreHorizontal size={16} />
                            </button>
                            <Show when={menuOpen()}>
                              <div class="absolute right-0 mt-1.5 w-40 bg-surface border border-white/[0.06] rounded-lg shadow-2xl z-50 py-1 animate-fade-in">
                                <button
                                  onMouseDown={() => {
                                    setMenuOpen(false);
                                    handleDelete();
                                  }}
                                  class="w-full flex items-center gap-2.5 px-3 py-2 text-sm text-red-400 hover:text-red-300 hover:bg-surface-hover transition-colors duration-150"
                                >
                                  Delete post
                                </button>
                              </div>
                            </Show>
                          </div>
                        </Show>
                      </div>
                    </div>

                    {/* Title + Content card */}
                    <div class="bg-surface/40 rounded-xl border border-white/[0.06] p-6">
                      {/* Title Suggestion */}
                      <Show when={isAuthor() && threadData().title_suggestion}>
                        <div class="mb-4 bg-zinc-800/50 border border-white/[0.06] rounded-lg p-4">
                          <div class="flex justify-between items-start mb-2">
                            <span class="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
                              <svg
                                class="w-3.5 h-3.5"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                              >
                                <path
                                  stroke-linecap="round"
                                  stroke-linejoin="round"
                                  stroke-width="2"
                                  d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
                                />
                              </svg>
                              AI title suggestion
                            </span>
                            <div class="flex gap-2">
                              <button
                                // oxlint-disable-next-line no-non-null-assertion
                                onMouseDown={() =>
                                  handleAcceptTitle(threadData().title_suggestion!)
                                }
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
                          <div class="text-lg font-semibold text-white">
                            {threadData().title_suggestion}
                          </div>
                        </div>
                      </Show>

                      {/* Title (CRDT for everyone, editable only for author) */}
                      <Show
                        when={titleCrdtField()}
                        fallback={
                          <h1 class="text-2xl font-semibold mb-4 leading-tight">
                            {threadData().title || 'Untitled'}
                          </h1>
                        }
                      >
                        {(field) => (
                          <CollaborativeEditor
                            field={field()}
                            content={threadData().title}
                            placeholder="Untitled"
                            class="text-2xl font-semibold mb-4 leading-tight [&_.ProseMirror]:outline-none"
                            editable={canEdit()}
                            singleLine
                            username={auth.user()?.username}
                            onUpdate={(text) => handleTitleChange(text)}
                          />
                        )}
                      </Show>

                      {/* Content Suggestion */}
                      <Show when={isAuthor() && threadData().content_suggestion}>
                        <div class="mb-4 bg-zinc-800/50 border border-white/[0.06] rounded-lg p-4">
                          <div class="flex justify-between items-start mb-2">
                            <span class="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
                              <svg
                                class="w-3.5 h-3.5"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                              >
                                <path
                                  stroke-linecap="round"
                                  stroke-linejoin="round"
                                  stroke-width="2"
                                  d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
                                />
                              </svg>
                              AI content suggestion
                            </span>
                            <div class="flex gap-2">
                              <button
                                // oxlint-disable-next-line no-non-null-assertion
                                onMouseDown={() =>
                                  handleAcceptContent(threadData().content_suggestion!)
                                }
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

                      {/* Content (CRDT for everyone, editable only for author) */}
                      <Show
                        when={contentCrdtField()}
                        fallback={
                          <div class="text-[15px] text-zinc-400 whitespace-pre-wrap leading-relaxed min-h-[80px]">
                            {threadData().content || 'No content yet...'}
                          </div>
                        }
                      >
                        {(field) => (
                          <CollaborativeEditor
                            field={field()}
                            content={threadData().content}
                            placeholder="Write something..."
                            class="text-[15px] text-zinc-300 focus-within:text-white leading-relaxed min-h-[120px] [&_.ProseMirror]:outline-none [&_.ProseMirror]:min-h-[120px]"
                            editable={canEdit()}
                            username={auth.user()?.username}
                            onUpdate={(text) => handleContentChange(text)}
                          />
                        )}
                      </Show>
                    </div>

                    {/* Actions row beneath the card */}
                    <div class="flex items-center justify-between mt-3 px-1">
                      <div class="flex items-center gap-4">
                        <Tooltip text="Reply" kbd="r">
                          <button
                            onMouseDown={() => {
                              const textarea = document.querySelector(
                                '#comment-textarea'
                              ) as HTMLTextAreaElement;
                              textarea?.focus();
                            }}
                            class="inline-flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-300 transition-colors duration-150"
                          >
                            <svg
                              class="w-4 h-4"
                              fill="none"
                              stroke="currentColor"
                              viewBox="0 0 24 24"
                            >
                              <path
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                stroke-width="1.5"
                                d="M12 20.25c4.97 0 9-3.694 9-8.25s-4.03-8.25-9-8.25S3 7.444 3 12c0 2.104.859 4.023 2.273 5.48.432.447.74 1.04.586 1.641a4.483 4.483 0 01-.923 1.785A5.969 5.969 0 006 21c1.282 0 2.47-.402 3.445-1.087.81.22 1.668.337 2.555.337z"
                              />
                            </svg>
                            {threadData().comments?.length || 0}
                          </button>
                        </Tooltip>
                      </div>

                      <Tooltip
                        text={isAuthor() ? 'Generate AI suggestions' : 'Only the author can Spookify this thread'}
                        position={isAuthor() ? 'bottom' : 'left'}
                      >
                        <SpookButton
                          loading={spookifySending() || spookifyJobLoading()}
                          loadingLabel="Processing..."
                          onClick={handleSpookify}
                          disabled={!isAuthor()}
                        >
                          Spookify
                        </SpookButton>
                      </Tooltip>
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

      <Show when={thread()?.id}>
        <ShareDialog
          threadId={thread()!.id}
          isOpen={shareOpen()}
          onClose={() => setShareOpen(false)}
        />
      </Show>
    </div>
  );
}
