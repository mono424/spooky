import { Show } from 'solid-js';
import { useAuth } from '../lib/auth';
import { SchemaDefinition } from '../schema.gen';
import { GetTable, TableModel } from '@spooky/client-solid';
import { db } from '../db';

type AugmentedComment = Omit<TableModel<GetTable<SchemaDefinition, 'comment'>>, 'author'> & {
  author?: TableModel<GetTable<SchemaDefinition, 'user'>>;
};

export function CommentBox(props: { comment: AugmentedComment }) {
  const spooky = db.getSpooky();
  const auth = useAuth();

  const isAdmin = () => {
    return auth.user()?.id === props.comment.author?.id;
  };

  const handleDelete = () => {
    if (confirm('CONFIRM_DELETION: Proceed with erasing this log entry?')) {
      spooky.delete('comment', props.comment.id);
    }
  };

  return (
    <div class="group relative bg-black border-l-2 border-gray-800 hover:border-white pl-4 py-2 transition-colors font-mono">
      {/* Author and Metadata Header */}
      <div class="flex justify-between items-start mb-2 text-[10px] uppercase tracking-wider">
        <div class="flex items-center gap-2">
          <span class="text-green-500 font-bold">&gt;</span>
          <span class="font-bold text-gray-300 group-hover:text-white">
            {props.comment.author?.username || 'UNKNOWN_USER'}
          </span>
          <span class="text-gray-600 hidden sm:inline">ID: {props.comment.id.slice(0, 8)}</span>
        </div>

        <span class="text-gray-600 group-hover:text-gray-400">
          {new Date(props.comment.created_at ?? 0).toLocaleDateString(undefined, {
            month: 'short',
            day: '2-digit',
            hour: '2-digit',
            minute: '2-digit',
          })}
        </span>
      </div>

      {/* Comment Content */}
      <p class="text-gray-400 text-sm whitespace-pre-wrap leading-relaxed mb-3 group-hover:text-gray-200">
        {props.comment.content}
      </p>

      {/* Admin Actions */}
      <Show when={isAdmin()}>
        <div class="flex justify-end gap-3 opacity-0 group-hover:opacity-100 transition-opacity">
          {/* Edit is a placeholder for now, styled as disabled/future feature if needed, 
                or we can just show Delete if Edit isn't implemented yet. 
                Keeping it minimalist based on your original code. 
            */}
          <button class="text-[10px] uppercase text-gray-500 hover:text-white hover:underline decoration-white underline-offset-2 transition-none">
            [ EDIT_DATA ]
          </button>
          <button
            onClick={handleDelete}
            class="text-[10px] uppercase text-red-900 hover:text-red-500 hover:bg-red-900/20 px-1 transition-none font-bold"
          >
            [ ERASE ]
          </button>
        </div>
      </Show>

      {/* Decorative vertical line connector for the 'tree' look */}
      <div class="absolute -left-[2px] top-0 bottom-0 w-[2px] bg-gray-800 group-hover:bg-white transition-colors"></div>
    </div>
  );
}
