import { Show } from 'solid-js';
import { useAuth } from '../lib/auth';
import type { SchemaDefinition, schema } from '../schema.gen';
import type { GetTable, TableModel} from '@spooky-sync/client-solid';
import { useDb } from '@spooky-sync/client-solid';
import { ProfilePicture } from './ProfilePicture';

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
    db.delete('comment', props.comment.id);
  };

  return (
    <div class="group flex gap-3 py-4 hover:bg-surface/20 -mx-2 px-2 rounded-xl transition-colors duration-150">
      {/* Avatar */}
      <ProfilePicture
        src={() => props.comment.author?.profile_picture}
        username={() => props.comment.author?.username}
        size="sm"
      />

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
