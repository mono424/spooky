import { Show } from "solid-js";
import { useShortcutsHelp } from "../lib/keyboard";

export function ShortcutsHelp() {
  const { isOpen, close } = useShortcutsHelp();

  return (
    <Show when={isOpen()}>
      <div 
        class="fixed inset-0 bg-black/80 backdrop-blur-sm z-[100] flex items-center justify-center p-4 font-mono"
        onClick={close}
      >
        <div 
          class="bg-black border-2 border-white w-full max-w-lg relative shadow-[0_0_20px_rgba(255,255,255,0.2)]"
          onClick={(e) => e.stopPropagation()}
        >
          {/* Header */}
          <div class="bg-white text-black px-4 py-2 flex justify-between items-center font-bold uppercase tracking-wider text-sm border-b-2 border-white">
            <span>[ KEYBOARD_CONTROLS ]</span>
            <button onClick={close} class="hover:bg-black hover:text-white px-2 transition-colors">
              [X]
            </button>
          </div>

          <div class="p-6 text-xs sm:text-sm">
             <div class="grid grid-cols-2 gap-x-8 gap-y-6">
                
                {/* Global Section */}
                <div>
                   <h3 class="text-gray-500 uppercase font-bold mb-3 border-b border-gray-800 pb-1">Global</h3>
                   <ul class="space-y-2">
                      <li class="flex justify-between">
                         <span>Show/Hide Help</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">?</span>
                      </li>
                      <li class="flex justify-between">
                         <span>Go Home</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">g h</span>
                      </li>
                      <li class="flex justify-between">
                         <span>Create Post</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">c</span>
                      </li>
                   </ul>
                </div>

                {/* Navigation Section */}
                <div>
                   <h3 class="text-gray-500 uppercase font-bold mb-3 border-b border-gray-800 pb-1">Navigation</h3>
                   <ul class="space-y-2">
                      <li class="flex justify-between">
                         <span>Select Next</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">j</span>
                      </li>
                      <li class="flex justify-between">
                         <span>Select Prev</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">k</span>
                      </li>
                      <li class="flex justify-between">
                         <span>Open Thread</span>
                         <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">Enter</span>
                      </li>
                   </ul>
                </div>

                 {/* Thread Detail Section */}
                 <div class="col-span-2">
                   <h3 class="text-gray-500 uppercase font-bold mb-3 border-b border-gray-800 pb-1">Thread Mode</h3>
                   <div class="grid grid-cols-2 gap-x-8">
                     <ul class="space-y-2">
                        <li class="flex justify-between">
                           <span>Reply / Focus</span>
                           <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">r</span>
                        </li>
                     </ul>
                      <ul class="space-y-2">
                        <li class="flex justify-between">
                           <span>Back / Blur</span>
                           <span class="bg-gray-900 border border-gray-700 px-1 text-white font-bold">Esc</span>
                        </li>
                     </ul>
                   </div>
                </div>

             </div>
          </div>

          {/* Footer */}
          <div class="border-t border-gray-800 p-2 text-center text-[10px] text-gray-500 uppercase">
             Press <span class="text-white">ESC</span> to close
          </div>

        </div>
      </div>
    </Show>
  );
}
