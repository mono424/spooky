import { createSignal, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { db } from "../db";
import { useAuth } from "../lib/auth";
import { Uuid } from "@spooky/client-solid";
import { RecordId } from "surrealdb";

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

      // Generate a record ID before creating
      const genId = Uuid.v4().toString().replace(/-/g, "");
      const threadId = `thread:${genId}`;
      await db.create(threadId, {
        title: title().trim(),
        content: content().trim(),
        author: new RecordId('user', user.id.toString().split(':')[1]),
        active: true,
      });

      handleClose();
      navigate(`/thread/${genId}`);
    } catch (err) {
      console.error("Failed to create thread:", err);
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
      {/* Styles for animation and scrollbar */}
      <style>{`
        @keyframes terminal-boot {
          0% { opacity: 0; transform: scale(0.95) translateY(10px); }
          100% { opacity: 1; transform: scale(1) translateY(0); }
        }
        .animate-terminal {
          animation: terminal-boot 0.2s cubic-bezier(0, 0, 0.2, 1) forwards;
        }
        
        /* Custom Scrollbar for the textarea/modal */
        ::-webkit-scrollbar { width: 8px; }
        ::-webkit-scrollbar-track { background: #000; border-left: 1px solid #333; }
        ::-webkit-scrollbar-thumb { background: #fff; border: 2px solid #000; }
        ::-webkit-scrollbar-thumb:hover { background: #ccc; }
      `}</style>

      <div class="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-[100] p-4">
        <div class="animate-terminal bg-black border-2 border-white w-full max-w-3xl flex flex-col shadow-[8px_8px_0px_0px_rgba(255,255,255,1)] max-h-[90vh]">
          
          {/* Header */}
          <div class="flex justify-between items-stretch border-b-2 border-white h-12 flex-shrink-0">
            <div class="flex items-center px-4 border-r-2 border-white bg-white text-black font-bold uppercase tracking-widest text-sm">
               [WRITE]
            </div>
            <div class="flex-grow flex items-center px-4 font-mono text-sm uppercase tracking-wider overflow-hidden whitespace-nowrap text-ellipsis">
               NEW_THREAD_BUFFER
            </div>
            <button
              onClick={handleClose}
              class="px-5 hover:bg-white hover:text-black border-l-2 border-white font-bold transition-none text-lg flex items-center justify-center"
              aria-label="Close"
            >
              ✕
            </button>
          </div>

          {/* Scrollable Content Area */}
          <div class="p-6 sm:p-8 overflow-y-auto">
            <form onSubmit={handleSubmit} class="space-y-6">
              
              {/* Title Input */}
              <div class="space-y-2">
                <div class="flex justify-between items-end">
                    <label for="title" class="text-xs uppercase font-bold tracking-wider">
                    &gt; Subject_Line:
                    </label>
                    <span class="text-[10px] text-gray-500 font-mono">
                        CHARS: {title().length}/200
                    </span>
                </div>
                <input
                  id="title"
                  type="text"
                  value={title()}
                  onInput={(e) => setTitle(e.currentTarget.value)}
                  required
                  maxlength="200"
                  class="w-full bg-black border-2 border-white px-4 py-3 text-white focus:outline-none focus:bg-white focus:text-black transition-none placeholder-gray-700 font-mono text-sm rounded-none"
                  placeholder="Enter subject..."
                  autocomplete="off"
                />
              </div>

              {/* Content Textarea */}
              <div class="space-y-2 flex-grow">
                <label for="content" class="block text-xs uppercase font-bold tracking-wider">
                  &gt; Body_Content:
                </label>
                <div class="relative">
                    <textarea
                        id="content"
                        value={content()}
                        onInput={(e) => setContent(e.currentTarget.value)}
                        required
                        rows="12"
                        class="w-full bg-black border-2 border-white p-4 text-white focus:outline-none focus:shadow-[4px_4px_0px_0px_rgba(255,255,255,1)] transition-none placeholder-gray-800 font-mono text-sm rounded-none resize-none leading-relaxed block"
                        placeholder="Begin transmission..."
                    />
                    {/* Blinking cursor decoration in bottom right corner if empty-ish */}
                    <div class="absolute bottom-2 right-4 pointer-events-none text-xs text-gray-600">
                        <span class="animate-pulse">█</span>
                    </div>
                </div>
              </div>

              <Show when={error()}>
                <div class="border border-red-500 text-red-500 p-3 text-xs font-mono uppercase bg-red-900/10">
                  <span class="font-bold">! WRITE_ERROR:</span> {error()}
                </div>
              </Show>

              {/* Action Buttons */}
              <div class="flex flex-col-reverse sm:flex-row justify-end gap-4 pt-2">
                <button
                  type="button"
                  onClick={handleClose}
                  class="px-6 py-3 border-2 border-transparent text-gray-500 hover:text-white uppercase font-bold text-xs tracking-wider hover:underline decoration-white underline-offset-4 transition-none"
                >
                  [ ABORT ]
                </button>
                <button
                  type="submit"
                  disabled={isLoading() || !title().trim() || !content().trim()}
                  class="bg-white text-black border-2 border-white px-8 py-3 uppercase font-bold hover:bg-black hover:text-white transition-none disabled:opacity-50 disabled:cursor-not-allowed text-sm tracking-widest"
                >
                  {isLoading() ? (
                    <span class="animate-pulse">TRANSMITTING...</span>
                  ) : (
                    "[ PUBLISH_THREAD ]"
                  )}
                </button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </Show>
  );
}