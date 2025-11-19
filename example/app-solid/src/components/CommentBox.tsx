import { createEffect, Show } from "solid-js";
import { useAuth } from "../lib/auth";
import { SchemaDefinition } from "../schema.gen";
import { GetTable, TableModel } from "@spooky/client-solid";
import { db } from "../db";

type AugmentedComment = Omit<
  TableModel<GetTable<SchemaDefinition, "comment">>,
  "author"
> & {
  author?: TableModel<GetTable<SchemaDefinition, "user">>;
};

export function CommentBox(props: { comment: AugmentedComment }) {
  const spooky = db.getSpooky();
  const auth = useAuth();

  const isAdmin = () => {
    return auth.user()?.id === props.comment.author?.id;
  };

  const handleDelete = () => {
    spooky.delete("comment", props.comment.id);
  };

  return (
    <div class="bg-white border border-gray-200 rounded-lg p-4 relative">
      <p class="text-gray-700 mb-2 whitespace-pre-wrap">
        {props.comment.content}
      </p>
      <div class="flex justify-between items-center text-sm text-gray-500">
        <span>By {props.comment.author?.username}</span>
        <span>
          {new Date(props.comment.created_at ?? 0).toLocaleDateString()}
        </span>
        <div class="flex gap-2 text-sm text-gray-500 w-32 justify-end">
          <Show when={isAdmin()}>
            <button>Edit</button>
            <button onClick={handleDelete}>Delete</button>
          </Show>
        </div>
      </div>
    </div>
  );
}
