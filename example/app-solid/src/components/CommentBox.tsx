import { Show } from 'solid-js';
import { useAuth } from '../lib/auth';
import { SchemaDefinition } from '../schema.gen';
import { GetTable, TableModel, useDb } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

type AugmentedComment = Omit<TableModel<GetTable<SchemaDefinition, 'comment'>>, 'author'> & {
  author?: TableModel<GetTable<SchemaDefinition, 'user'>>;
};

export function CommentBox(props: { comment: AugmentedComment }) {
  const db = useDb<typeof schema>();
  const auth = useAuth();

  const isAdmin = () => {
    return auth.user()?.id === props.comment.author?.id;
  };

  const handleDelete = () => {
    if (confirm('Are you sure you want to delete this reply?')) {
      db.delete('comment', props.comment.id);
    }
  };

  const authorInitial = () => {
    const name = props.comment.author?.username;
    return name ? name.charAt(0).toUpperCase() : '?';
  };

  return (
    <div class="group flex gap-3 py-4 hover:bg-surface/20 -mx-2 px-2 rounded-xl transition-colors duration-150">
      {/* Avatar */}
      <div class="w-8 h-8 rounded-full bg-accent/15 text-accent flex items-center justify-center text-xs font-semibold flex-shrink-0 mt-0.5">
        {authorInitial()}
      </div>

      <div class="flex-1 min-w-0">
        {/* Name + time + actions */}
        <div class="flex items-center gap-2 mb-1">
          <span class="text-sm font-medium text-zinc-200">
            {props.comment.author?.username || 'Unknown'}
          </span>
          <span class="text-zinc-700">&middot;</span>
          <span class="text-xs text-zinc-600">
            {new Date(props.comment.created_at ?? 0).toLocaleDateString(undefined, {
              month: 'short',
              day: '2-digit',
              hour: '2-digit',
              minute: '2-digit',
            })}
          </span>

          <Show when={isAdmin()}>
            <div class="ml-auto flex gap-3 opacity-0 group-hover:opacity-100 transition-opacity duration-150">
              <button class="text-xs text-zinc-600 hover:text-zinc-300 transition-colors duration-150">
                Edit
              </button>
              <button
                onClick={handleDelete}
                class="text-xs text-zinc-600 hover:text-red-400 transition-colors duration-150"
              >
                Delete
              </button>
            </div>
          </Show>
        </div>

        {/* Content */}
        <p class="text-[14px] text-zinc-400 whitespace-pre-wrap leading-relaxed">
          {props.comment.content}
        </p>
      </div>
    </div>
  );
}
