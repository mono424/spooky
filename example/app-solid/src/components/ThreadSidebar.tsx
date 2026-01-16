import { For } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../db";
import { useQuery } from "@spooky/client-solid";

interface ThreadSidebarProps {
  activeThreadId?: string;
}

export function ThreadSidebar(props: ThreadSidebarProps) {
  const navigate = useNavigate();

  const threadsResult = useQuery(db, () => {
    return db
      .query("thread")
      .orderBy("title", "asc")
      .limit(20)
      .build();
  });

  const threads = () => threadsResult.data() || [];

  const handleThreadClick = (threadId: string) => {
    const id = threadId.split(":")[1];
    navigate(`/thread/${id}`);
  };

  const isActive = (threadId: string) => {
    const id = threadId.split(":")[1];
    return id === props.activeThreadId;
  };

  return (
    <div class="w-56 border-r border-gray-800 overflow-y-auto h-full flex-shrink-0">
      <div class="sticky top-0 bg-black border-b border-gray-800 px-4 py-3">
        <h3 class="text-[10px] font-bold uppercase text-gray-500 tracking-wider">
          THREADS
        </h3>
      </div>
      
      <div class="py-2">
        <For
          each={threads()}
          fallback={
            <div class="px-4 py-8 text-center">
              <div class="text-[10px] text-gray-600 uppercase">
                NO_THREADS
              </div>
            </div>
          }
        >
          {(thread) => (
            <button
              onClick={() => handleThreadClick(thread.id)}
              class={`w-full text-left px-4 py-2 text-xs uppercase font-mono transition-none border-l-2 ${
                isActive(thread.id)
                  ? "border-white text-white bg-white/5"
                  : "border-transparent text-gray-500 hover:text-gray-300 hover:border-gray-700"
              }`}
            >
              <div class="truncate">
                {isActive(thread.id) && <span class="mr-1">&gt;</span>}
                {thread.title || "UNTITLED"}
              </div>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}
