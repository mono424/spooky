import { createSignal } from "solid-js";
import { db } from "../db";
import { useAuth } from "../lib/auth";
import { Model, Snapshot } from "@spooky/client-solid";
import { Thread } from "../schema.gen";

interface CommentFormProps {
  thread: Snapshot<Model<Thread>>;
  onCommentAdded?: () => void;
}

export function CommentForm(props: CommentFormProps) {
  const auth = useAuth();
  const [content, setContent] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!content().trim() || isLoading()) return;

    setIsLoading(true);
    try {
      const user = auth.user();
      if (!user) {
        throw new Error("You must be logged in to post a comment");
      }

      await db.query.comment.createRemote({
        thread_id: props.thread.id,
        content: content().trim(),
        author: user.id,
        created_at: new Date(),
      });

      setContent("");
      props.onCommentAdded?.();
    } catch (error) {
      console.error("Failed to create comment:", error);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} class="space-y-3">
      <div>
        <textarea
          value={content()}
          onInput={(e) => setContent(e.currentTarget.value)}
          placeholder="Write a comment..."
          rows="3"
          class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 resize-none"
          required
        />
      </div>
      <div class="flex justify-end">
        <button
          type="submit"
          disabled={isLoading() || !content().trim()}
          class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {isLoading() ? "Posting..." : "Post Comment"}
        </button>
      </div>
    </form>
  );
}
