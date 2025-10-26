import {
  createResource,
  createSignal,
  For,
  Show,
  onCleanup,
  onMount,
} from "solid-js";
import { useNavigate, useParams } from "@solidjs/router";
import { db } from "../db";
import { CommentForm } from "./CommentForm";
import { Model, RecordId } from "db-solid";
import { Thread, Comment } from "../schema.gen";

// Type for transformed comment with nested author
type TransformedComment = Omit<Comment, "author"> & {
  author: { id: string };
};

export function ThreadDetail() {
  const params = useParams();
  const navigate = useNavigate();

  const [thread, setThread] = createSignal<Model<Thread> | null>(null);
  const [comments, setComments] = createSignal<Model<Comment>[]>([]);

  onMount(async () => {
    const threadLiveQuery = await db.query.thread.liveQuery(
      {
        where: {
          id: new RecordId("thread", params.id),
        },
      },
      (thread) => {
        if (thread.length > 0) setThread(thread[0]);
      }
    );

    const commentsLiveQuery = await db.query.comment.liveQuery(
      {
        where: {
          thread_id: new RecordId("thread", params.id),
        },
      },
      (comments) => {
        setComments(comments);
      }
    );

    onCleanup(() => {
      threadLiveQuery.kill();
      commentsLiveQuery.kill();
    });
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
                <span>By {threadData().author}</span>
                <span>
                  {new Date(threadData().created_at ?? 0).toLocaleDateString()}
                </span>
              </div>
            </div>

            {/* Comments Section */}
            <div class="space-y-4">
              <h2 class="text-xl font-semibold">
                Comments ({comments()?.length})
              </h2>

              {/* Comment Form */}
              <div class="bg-gray-50 border border-gray-200 rounded-lg p-4">
                <CommentForm threadId={threadData().id.toString()} />
              </div>

              {/* Comments List */}
              <div class="space-y-3">
                <For
                  each={comments() ?? []}
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
                        <span>By {comment.author.id}</span>
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
