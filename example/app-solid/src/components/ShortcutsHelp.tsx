import { Show } from 'solid-js';
import { useKeyboard, useShortcutsHelp } from '../lib/keyboard';

export function ShortcutsHelp() {
  const { isOpen, close } = useShortcutsHelp();

  useKeyboard({
    Escape: () => {
      if (isOpen()) close();
    },
  });

  return (
    <Show when={isOpen()}>
      <div
        class="fixed inset-0 bg-black/60 backdrop-blur-sm z-[100] flex items-center justify-center p-4"
        onClick={close}
      >
        <div
          class="animate-slide-up bg-surface border border-white/[0.06] rounded-xl w-full max-w-lg shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          {/* Header */}
          <div class="flex justify-between items-center px-6 pt-6 pb-4">
            <h2 class="text-lg font-semibold">Keyboard shortcuts</h2>
            <button
              onClick={close}
              class="text-zinc-500 hover:text-white transition-colors duration-150 p-1"
            >
              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          <div class="px-6 pb-6 text-sm">
            <div class="grid grid-cols-2 gap-x-8 gap-y-6">
              {/* Global Section */}
              <div>
                <h3 class="text-xs font-medium text-zinc-500 mb-3 pb-1 border-b border-white/[0.06]">
                  Global
                </h3>
                <ul class="space-y-2.5">
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Show help</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      ?
                    </kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Go home</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      g h
                    </kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Create post</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      c
                    </kbd>
                  </li>
                </ul>
              </div>

              {/* Navigation Section */}
              <div>
                <h3 class="text-xs font-medium text-zinc-500 mb-3 pb-1 border-b border-white/[0.06]">
                  Navigation
                </h3>
                <ul class="space-y-2.5">
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Next</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      j
                    </kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Previous</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      k
                    </kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400">Open</span>
                    <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                      Enter
                    </kbd>
                  </li>
                </ul>
              </div>

              {/* Thread Section */}
              <div class="col-span-2">
                <h3 class="text-xs font-medium text-zinc-500 mb-3 pb-1 border-b border-white/[0.06]">
                  Thread
                </h3>
                <div class="grid grid-cols-2 gap-x-8">
                  <ul class="space-y-2.5">
                    <li class="flex justify-between items-center">
                      <span class="text-zinc-400">Reply</span>
                      <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                        r
                      </kbd>
                    </li>
                  </ul>
                  <ul class="space-y-2.5">
                    <li class="flex justify-between items-center">
                      <span class="text-zinc-400">Back</span>
                      <kbd class="bg-zinc-800 border border-white/[0.06] rounded px-2 py-0.5 text-xs text-zinc-300 font-mono">
                        Esc
                      </kbd>
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          </div>

          {/* Footer */}
          <div class="border-t border-white/[0.06] px-6 py-3 text-center text-xs text-zinc-600">
            Press <kbd class="text-zinc-400">Esc</kbd> to close
          </div>
        </div>
      </div>
    </Show>
  );
}
