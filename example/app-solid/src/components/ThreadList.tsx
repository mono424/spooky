import { For } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../db";
import { useQuery } from "@spooky/client-solid";

export function ThreadList() {
  const navigate = useNavigate();

  const threadsResult = useQuery(db, () =>
    db
      .query("thread")
      .related("author")
      .orderBy("created_at", "desc")
      .orderBy("id", "asc")
      .limit(10)
      .build()
  );
  
  const threads = () => threadsResult.data() || [];

  const handleThreadClick = (threadId: string) => {
    navigate(`/thread/${threadId}`);
  };

  return (
    <div class="w-full font-mono">
      {/* Action Bar: 
        Minimalist style to blend with the main Layout header. 
        Removed the heavy bottom border and large H1.
      */}
      <div class="flex justify-between items-center mb-8 pt-2">
        <div class="text-xs sm:text-sm text-gray-500 font-mono uppercase tracking-wider">
           <span class="text-green-500 font-bold mr-2">root@spooky:~/threads$</span>
           <span class="animate-pulse">_</span>
        </div>
        
        <button
          onClick={() => navigate("/create-thread")}
          class="bg-white text-black border-2 border-white px-4 py-2 uppercase font-bold text-xs hover:bg-black hover:text-white transition-none"
        >
          [ + WRITE_NEW ]
        </button>
      </div>

      {/* List Section */}
      <div class="space-y-4">
        <For
          each={threads()}
          fallback={
            <div class="border-2 border-dashed border-gray-800 p-12 text-center opacity-50">
              <pre class="text-xs mb-4 text-gray-500 whitespace-pre leading-none font-mono">
              {`
   _______
-  /   ___  \\  -
-  |  /   \\  |  -
-  |  |   |  |  -
-  |  \\___/  |  -
-  \\________/  -
              `}
              </pre>
              <div class="uppercase tracking-widest text-xs font-bold">
                &lt; NULL_RESPONSE /&gt;
              </div>
              <p class="text-[10px] mt-2">
                Directory is empty. Execute write command.
              </p>
            </div>
          }
        >
          {(thread) => (
            <div
              onClick={() => handleThreadClick(thread.id.split(":")[1])}
              class="border border-white/40 p-5 cursor-pointer hover:border-white hover:bg-white/5 transition-none group relative block"
            >
              {/* ASCII Corner markers that appear on hover */}
              <div class="hidden group-hover:block absolute -top-3 -left-2 text-white text-[10px]">+</div>
              <div class="hidden group-hover:block absolute -top-3 -right-2 text-white text-[10px]">+</div>
              <div class="hidden group-hover:block absolute -bottom-3 -left-2 text-white text-[10px]">+</div>
              <div class="hidden group-hover:block absolute -bottom-3 -right-2 text-white text-[10px]">+</div>

              <div class="flex flex-col sm:flex-row sm:justify-between sm:items-start gap-2 mb-2">
                <h2 class="text-lg font-bold uppercase tracking-wide group-hover:text-green-400">
                  {thread.title}
                </h2>
                <div class="text-[10px] text-gray-500 border border-gray-800 px-2 py-1 bg-black">
                  ID: {thread.id.split(":")[1].slice(0, 8)}
                </div>
              </div>
              
              <p class="text-sm text-gray-400 mb-4 font-mono line-clamp-2 leading-relaxed pl-2 border-l-2 border-gray-800 group-hover:border-white group-hover:text-gray-300">
                {thread.content}
              </p>
              
              <div class="flex justify-between items-center text-[10px] uppercase text-gray-600 font-bold tracking-wider">
                <div class="flex items-center gap-2">
                  <span>USR:</span>
                  <span class="text-gray-400 group-hover:text-white">{thread.author?.username || "ANON"}</span>
                </div>
                <div>
                   {new Date(thread.created_at ?? 0).toLocaleDateString(undefined, {
                      month: 'short',
                      day: '2-digit',
                      hour: '2-digit',
                      minute: '2-digit'
                    })}
                </div>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}