import { createResource, For } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../lib/db";
import { useAuth } from "../lib/auth";

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

export function ThreadList() {
  const navigate = useNavigate();
  const auth = useAuth();

  const [threads, { refetch }] = createResource(async () => {
    try {
      const result = await db.queryLocal<{ result: Thread[] }>(`
        SELECT 
          id,
          title,
          content,
          author.id as author_id,
          author.username as author_username,
          created_at
        FROM thread
        ORDER BY created_at DESC
      `);

      return (
        result.result?.map((thread) => ({
          ...thread,
          author: {
            id: thread.author_id,
            username: thread.author_username,
          },
        })) || []
      );
    } catch (error) {
      console.error("Failed to fetch threads:", error);
      return [];
    }
  });

  const handleThreadClick = (threadId: string) => {
    navigate(`/thread/${threadId}`);
  };

  return (
    <div class="max-w-4xl mx-auto p-4">
      <div class="flex justify-between items-center mb-6">
        <h1 class="text-3xl font-bold">Threads</h1>
        <button
          onClick={() => navigate("/create-thread")}
          class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700"
        >
          New Thread
        </button>
      </div>

      <div class="space-y-4">
        <For
          each={threads()}
          fallback={
            <div class="text-center py-8 text-gray-500">
              No threads found. Create the first one!
            </div>
          }
        >
          {(thread) => (
            <div
              onClick={() => handleThreadClick(thread.id)}
              class="bg-white border border-gray-200 rounded-lg p-4 hover:shadow-md cursor-pointer transition-shadow"
            >
              <h2 class="text-xl font-semibold mb-2">{thread.title}</h2>
              <p class="text-gray-600 mb-3 line-clamp-3">{thread.content}</p>
              <div class="flex justify-between items-center text-sm text-gray-500">
                <span>By {thread.author.username}</span>
                <span>{new Date(thread.created_at).toLocaleDateString()}</span>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
