import type { JSX} from 'solid-js';
import { createSignal, Show, onCleanup } from 'solid-js';

interface TooltipProps {
  text: string;
  kbd?: string;
  position?: 'top' | 'bottom';
  children: JSX.Element;
}

export function Tooltip(props: TooltipProps) {
  const [visible, setVisible] = createSignal(false);
  let timeout: ReturnType<typeof setTimeout>;
  const pos = () => props.position ?? 'bottom';

  const show = () => {
    timeout = setTimeout(() => setVisible(true), 400);
  };
  const hide = () => {
    clearTimeout(timeout);
    setVisible(false);
  };

  onCleanup(() => clearTimeout(timeout));

  return (
    <div
      class="relative inline-flex"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocusIn={show}
      onFocusOut={hide}
    >
      {props.children}
      <Show when={visible()}>
        <div
          class={`absolute left-1/2 -translate-x-1/2 z-[200] pointer-events-none ${
            pos() === 'top' ? 'bottom-full mb-2' : 'top-full mt-2'
          }`}
          style="animation: tooltip-enter 0.18s cubic-bezier(0.16, 1, 0.3, 1) forwards"
        >
          <div
            class="flex items-center gap-2 whitespace-nowrap rounded-xl px-3 py-1.5"
            style="background: rgba(255, 255, 255, 0.06); backdrop-filter: blur(20px) saturate(1.4); -webkit-backdrop-filter: blur(20px) saturate(1.4); border: 1px solid rgba(255, 255, 255, 0.1); box-shadow: 0 4px 24px rgba(0, 0, 0, 0.3), inset 0 0.5px 0 rgba(255, 255, 255, 0.12);"
          >
            <span class="text-[11px] font-medium text-zinc-200/90">{props.text}</span>
            <Show when={props.kbd}>
              <kbd
                class="text-[10px] font-mono leading-none rounded-md px-1.5 py-0.5 text-zinc-300"
                style="background: rgba(255, 255, 255, 0.08); border: 1px solid rgba(255, 255, 255, 0.1); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), inset 0 0.5px 0 rgba(255, 255, 255, 0.1);"
              >
                {props.kbd}
              </kbd>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
