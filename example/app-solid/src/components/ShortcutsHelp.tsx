import { Show } from 'solid-js';
import { createHotkey, useShortcutsHelp } from '../lib/keyboard';
import { Tooltip } from './Tooltip';

export function ShortcutsHelp() {
  const { isOpen, close } = useShortcutsHelp();

  createHotkey('Escape', () => close(), () => ({ enabled: isOpen(), ignoreInputs: false }));

  const kbdClass = "text-[11px] font-mono text-zinc-300 rounded-md px-2 py-0.5 leading-none";
  const kbdStyle = "background: rgba(255, 255, 255, 0.08); border: 1px solid rgba(255, 255, 255, 0.1); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), inset 0 0.5px 0 rgba(255, 255, 255, 0.1);";

  return (
    <Show when={isOpen()}>
      <div
        class="fixed inset-0 bg-black/50 backdrop-blur-md z-[100] flex items-center justify-center p-4"
        onClick={close}
      >
        <div
          class="animate-slide-up w-full max-w-lg rounded-2xl overflow-hidden"
          style="background: rgba(255, 255, 255, 0.05); backdrop-filter: blur(40px) saturate(1.5); -webkit-backdrop-filter: blur(40px) saturate(1.5); border: 1px solid rgba(255, 255, 255, 0.1); box-shadow: 0 8px 48px rgba(0, 0, 0, 0.4), inset 0 0.5px 0 rgba(255, 255, 255, 0.12);"
          onClick={(e) => e.stopPropagation()}
        >
          {/* Header */}
          <div class="flex justify-between items-center px-6 pt-6 pb-4">
            <h2 class="text-base font-semibold text-zinc-100">Keyboard shortcuts</h2>
            <Tooltip text="Close" kbd="Esc" position="bottom">
              <button
                onClick={close}
                class="text-zinc-500 hover:text-white transition-colors duration-150 p-1 rounded-lg hover:bg-white/[0.06]"
              >
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </Tooltip>
          </div>

          <div class="px-6 pb-6 text-sm">
            <div class="grid grid-cols-2 gap-x-8 gap-y-5">
              {/* Global Section */}
              <div>
                <h3 class="text-[10px] font-medium text-zinc-500 uppercase tracking-widest mb-3 pb-1.5" style="border-bottom: 1px solid rgba(255, 255, 255, 0.06);">
                  Global
                </h3>
                <ul class="space-y-2.5">
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Show help</span>
                    <kbd class={kbdClass} style={kbdStyle}>?</kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Go home</span>
                    <kbd class={kbdClass} style={kbdStyle}>g h</kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Create post</span>
                    <kbd class={kbdClass} style={kbdStyle}>c</kbd>
                  </li>
                </ul>
              </div>

              {/* Navigation Section */}
              <div>
                <h3 class="text-[10px] font-medium text-zinc-500 uppercase tracking-widest mb-3 pb-1.5" style="border-bottom: 1px solid rgba(255, 255, 255, 0.06);">
                  Navigation
                </h3>
                <ul class="space-y-2.5">
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Next</span>
                    <kbd class={kbdClass} style={kbdStyle}>j</kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Previous</span>
                    <kbd class={kbdClass} style={kbdStyle}>k</kbd>
                  </li>
                  <li class="flex justify-between items-center">
                    <span class="text-zinc-400 text-[13px]">Open</span>
                    <kbd class={kbdClass} style={kbdStyle}>Enter</kbd>
                  </li>
                </ul>
              </div>

              {/* Thread Section */}
              <div class="col-span-2">
                <h3 class="text-[10px] font-medium text-zinc-500 uppercase tracking-widest mb-3 pb-1.5" style="border-bottom: 1px solid rgba(255, 255, 255, 0.06);">
                  Thread
                </h3>
                <div class="grid grid-cols-2 gap-x-8">
                  <ul class="space-y-2.5">
                    <li class="flex justify-between items-center">
                      <span class="text-zinc-400 text-[13px]">Reply</span>
                      <kbd class={kbdClass} style={kbdStyle}>r</kbd>
                    </li>
                  </ul>
                  <ul class="space-y-2.5">
                    <li class="flex justify-between items-center">
                      <span class="text-zinc-400 text-[13px]">Back</span>
                      <kbd class={kbdClass} style={kbdStyle}>Esc</kbd>
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          </div>

          {/* Footer */}
          <div class="px-6 py-3 text-center text-[11px] text-zinc-600" style="border-top: 1px solid rgba(255, 255, 255, 0.06);">
            Press <kbd class={kbdClass} style={kbdStyle}>Esc</kbd> to close
          </div>
        </div>
      </div>
    </Show>
  );
}
