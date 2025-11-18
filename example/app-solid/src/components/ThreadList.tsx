import { For } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../db";
import { useQuery } from "@spooky/client-solid";

export function ThreadList() {
  const navigate = useNavigate();

  const threadsResult = useQuery(() =>
    db
      .query("thread")
      .related("author")
      .orderBy("created_at", "desc")
      .limit(100)
      .buildCustom()
  );
  const threads = () => threadsResult.data || [];

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
              No threads found. Create the first one! {threads().length}
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
                <span>
                  {new Date(thread.created_at ?? 0).toLocaleDateString()}
                </span>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
