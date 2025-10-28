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
import { useAuth } from "../lib/auth";

export function ThreadDetail() {
  const auth = useAuth();
  const params = useParams();
  const navigate = useNavigate();
  const [commentFilter, setCommentFilter] = createSignal<"all" | "mine">("all");

  // Create a reactive query that rebuilds when commentFilter changes
  const threadQuery = () => {
    const query = db.query.thread
      .find({
        id: params.id,
      })
      .related("author")
      .related("comments", (q) => {
        // q = q.related("author");
        if (commentFilter() === "mine" && auth.user()?.id) {
          return q.where({ author: auth.user()!.id });
        }
        return q;
      })
      .one();

    return query;
  };

  const [thread, setThread] = createSignal<any>(null);

  // useQuery will automatically handle cleanup and re-execution when threadQuery changes
  useQuery(threadQuery, setThread);

  createEffect(() => {
    const t = thread();
    console.log("thread", t);
    console.log("thread.comments", t?.comments);
    console.log("thread.comments length", t?.comments?.length);
    console.log("commentFilter", commentFilter());
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
                <span>
                  By{" "}
                  {(() => {
                    const author = threadData().author;
                    // Handle both array (from subquery) and object formats
                    const authorObj = Array.isArray(author)
                      ? author[0]
                      : author;
                    if (
                      authorObj &&
                      typeof authorObj === "object" &&
                      "username" in authorObj
                    ) {
                      return (authorObj as any).username;
                    }
                    return "Unknown";
                  })()}
                </span>
                <span>
                  {new Date(threadData().created_at ?? 0).toLocaleDateString()}
                </span>
              </div>
            </div>

            {/* Comments Section */}
            <div class="space-y-4">
              <div class="flex justify-between items-center">
                <h2 class="text-xl font-semibold">
                  Comments ({threadData().comments?.length})
                </h2>
                <Show when={auth.user()}>
                  <div class="flex gap-2">
                    <button
                      onClick={() => setCommentFilter("all")}
                      class={`px-3 py-1 rounded ${
                        commentFilter() === "all"
                          ? "bg-blue-600 text-white"
                          : "bg-gray-200 text-gray-700"
                      }`}
                    >
                      All
                    </button>
                    <button
                      onClick={() => setCommentFilter("mine")}
                      class={`px-3 py-1 rounded ${
                        commentFilter() === "mine"
                          ? "bg-blue-600 text-white"
                          : "bg-gray-200 text-gray-700"
                      }`}
                    >
                      My Comments
                    </button>
                  </div>
                </Show>
              </div>

              {/* Comment Form */}
              <div class="bg-gray-50 border border-gray-200 rounded-lg p-4">
                <CommentForm thread={threadData} />
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
                          {(() => {
                            const author = comment.author;
                            if (typeof author === "string") return author;
                            // Handle array (from subquery) and object formats
                            const authorObj = Array.isArray(author)
                              ? author[0]
                              : author;
                            return authorObj?.username ?? "Unknown";
                          })()}
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
