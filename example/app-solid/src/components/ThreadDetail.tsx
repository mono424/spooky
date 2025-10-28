import {
  createResource,
  createSignal,
  createEffect,
  For,
  Show,
  onCleanup,
  onMount,
} from "solid-js";
import { useNavigate, useParams } from "@solidjs/router";
import { db } from "../db";
import { CommentForm } from "./CommentForm";
import { useQuery, InferQueryModel } from "@spooky/client-solid";

export function ThreadDetail() {
  const params = useParams();
  const navigate = useNavigate();

  const threadQuery = db.query.thread
    .find({
      id: params.id,
    })
    .related("")
    .related("author", (q) => q.select("content", "created_at"))
    .related("comments")
    .one();

  const [thread, setThread] = createSignal<InferQueryModel<typeof threadQuery>>(
    null as any
  );
  useQuery(threadQuery, setThread);

  createEffect(() => {
    console.log("thread", thread());
  });

  onCleanup(() => {
    threadQuery.kill();
  });

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
                <span>By {threadData().author?.username ?? "Unknown"}</span>
                <span>
                  {new Date(threadData().created_at ?? 0).toLocaleDateString()}
                </span>
              </div>
            </div>

            {/* Comments Section */}
            <div class="space-y-4">
              <h2 class="text-xl font-semibold">
                Comments ({threadData().comments?.length})
              </h2>

              {/* Comment Form */}
              <div class="bg-gray-50 border border-gray-200 rounded-lg p-4">
                <CommentForm thread={threadData()} />
              </div>

              {/* Comments List */}
              <div class="space-y-3">
                <For
                  each={threadData().comments ?? []}
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
                        <span>
                          By{" "}
                          {typeof comment.author === "string"
                            ? comment.author
                            : comment.author?.username ?? "Unknown"}
                        </span>
                        <span>
                          {new Date(
                            comment.created_at ?? 0
                          ).toLocaleDateString()}
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
