import { createSignal, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../db";
import { useAuth } from "../lib/auth";
import { snapshot } from "@spooky/client-solid";

interface CreateThreadDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CreateThreadDialog(props: CreateThreadDialogProps) {
  const navigate = useNavigate();
  const auth = useAuth();
  const [title, setTitle] = createSignal("");
  const [content, setContent] = createSignal("");
  const [error, setError] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!title().trim() || !content().trim() || isLoading()) return;

    setError("");
    setIsLoading(true);

    try {
      const user = auth.user();
      if (!user) {
        throw new Error("You must be logged in to create a thread");
      }

      const [thread] = await db.query.thread.create({
        title: title().trim(),
        content: content().trim(),
        author: user.id,
        created_at: new Date(),
      });

      if (thread) {
        const threadId = thread.id.id.toString();
        props.onClose();
        navigate(`/thread/${threadId}`);
      } else {
        throw new Error("Failed to create thread");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create thread");
    } finally {
      setIsLoading(false);
    }
  };

  const handleClose = () => {
    setTitle("");
    setContent("");
    setError("");
    props.onClose();
  };

  return (
    <Show when={props.isOpen}>
      <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
        <div class="bg-white rounded-lg p-6 w-full max-w-2xl mx-4 max-h-[90vh] overflow-y-auto">
          <div class="flex justify-between items-center mb-4">
            <h2 class="text-xl font-bold">Create New Thread</h2>
            <button
              onClick={handleClose}
              class="text-gray-500 hover:text-gray-700"
            >
              ✕
            </button>
          </div>

          <form onSubmit={handleSubmit} class="space-y-4">
            <div>
              <label for="title" class="block text-sm font-medium mb-1">
                Title
              </label>
              <input
                id="title"
                type="text"
                value={title()}
                onInput={(e) => setTitle(e.currentTarget.value)}
                required
                maxlength="200"
                class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="Enter thread title"
              />
            </div>

            <div>
              <label for="content" class="block text-sm font-medium mb-1">
                Content
              </label>
              <textarea
                id="content"
                value={content()}
                onInput={(e) => setContent(e.currentTarget.value)}
                required
                rows="8"
                class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 resize-none"
                placeholder="Write your thread content..."
              />
            </div>

            <Show when={error()}>
              <div class="text-red-600 text-sm">{error()}</div>
            </Show>

            <div class="flex justify-end space-x-3">
              <button
                type="button"
                onClick={handleClose}
                class="px-4 py-2 border border-gray-300 rounded-md hover:bg-gray-50"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isLoading() || !title().trim() || !content().trim()}
                class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isLoading() ? "Creating..." : "Create Thread"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </Show>
  );
}
