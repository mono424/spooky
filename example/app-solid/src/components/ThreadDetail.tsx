import { createResource, For } from "solid-js";
import { useNavigate, useParams } from "solid-router";
import { db } from "../lib/db";
import { useAuth } from "../lib/auth";
import { CommentForm } from "./CommentForm";

interface Thread {
  id: string;
  title: string;
  content: string;
  author: {
    id: string;
    username: string;
  };
  created_at: Date;
}

interface Comment {
  id: string;
  content: string;
  author: {
    id: string;
    username: string;
  };
  created_at: Date;
}

export function ThreadDetail() {
  const params = useParams();
  const navigate = useNavigate();
  const auth = useAuth();

  const [thread, { refetch: refetchThread }] = createResource(
    () => params.id,
    async (threadId) => {
      try {
        const result = await db.queryLocal<{ result: Thread[] }>(
          `
          SELECT 
            id,
            title,
            content,
            author.id as author_id,
            author.username as author_username,
            created_at
          FROM thread
          WHERE id = $thread_id
        `,
          { thread_id: threadId }
        );

        if (result.result && result.result.length > 0) {
          const thread = result.result[0];
          return {
            ...thread,
            author: {
              id: thread.author_id,
              username: thread.author_username,
            },
          };
        }
        return null;
      } catch (error) {
        console.error("Failed to fetch thread:", error);
        return null;
      }
    }
  );

  const [comments, { refetch: refetchComments }] = createResource(
    () => params.id,
    async (threadId) => {
      try {
        const result = await db.queryLocal<{ result: Comment[] }>(
          `
          SELECT 
            id,
            content,
            author.id as author_id,
            author.username as author_username,
            created_at
          FROM comment
          WHERE thread_id = $thread_id
          ORDER BY created_at ASC
        `,
          { thread_id: threadId }
        );

        return (
          result.result?.map((comment) => ({
            ...comment,
            author: {
              id: comment.author_id,
              username: comment.author_username,
            },
          })) || []
        );
      } catch (error) {
        console.error("Failed to fetch comments:", error);
        return [];
      }
    }
  );

  const handleCommentAdded = () => {
    refetchComments();
  };

  const handleBack = () => {
    navigate("/");
  };

  return (
    <div class="max-w-4xl mx-auto p-4">
      <button
        onClick={handleBack}
        class="mb-4 text-blue-600 hover:text-blue-800"
      >
        ← Back to Threads
      </button>

      <Show
        when={thread()}
        fallback={
          <div class="text-center py-8 text-gray-500">Thread not found</div>
        }
      >
        {(threadData) => (
          <div class="space-y-6">
            {/* Thread Content */}
            <div class="bg-white border border-gray-200 rounded-lg p-6">
              <h1 class="text-2xl font-bold mb-3">{threadData().title}</h1>
              <p class="text-gray-700 mb-4 whitespace-pre-wrap">
                {threadData().content}
              </p>
              <div class="flex justify-between items-center text-sm text-gray-500 border-t pt-3">
                <span>By {threadData().author.username}</span>
                <span>
                  {new Date(threadData().created_at).toLocaleDateString()}
                </span>
              </div>
            </div>

            {/* Comments Section */}
            <div class="space-y-4">
              <h2 class="text-xl font-semibold">
                Comments ({comments().length})
              </h2>

              {/* Comment Form */}
              <div class="bg-gray-50 border border-gray-200 rounded-lg p-4">
                <CommentForm
                  threadId={threadData().id}
                  onCommentAdded={handleCommentAdded}
                />
              </div>

              {/* Comments List */}
              <div class="space-y-3">
                <For
                  each={comments()}
                  fallback={
                    <div class="text-center py-4 text-gray-500">
                      No comments yet. Be the first to comment!
                    </div>
                  }
                >
                  {(comment) => (
                    <div class="bg-white border border-gray-200 rounded-lg p-4">
                      <p class="text-gray-700 mb-2 whitespace-pre-wrap">
                        {comment.content}
                      </p>
                      <div class="flex justify-between items-center text-sm text-gray-500">
                        <span>By {comment.author.username}</span>
                        <span>
                          {new Date(comment.created_at).toLocaleDateString()}
                        </span>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
