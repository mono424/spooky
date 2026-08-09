import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { useDevTools } from '../context/DevToolsContext';
import type { Sp00kyFrame } from '../types/devtools';

/**
 * The toolbar's connection indicator, and the picker for WHICH client the panel
 * inspects. A tab can run several Sp00ky clients — the main document plus any
 * iframe that embeds the app — and every tab in this panel shows exactly one of
 * them, so the choice has to live somewhere always visible.
 *
 * It stays a plain dot while the main document is the only client (the common
 * case), and grows a label + caret as soon as there is something to choose
 * between.
 */
export function FrameSelect() {
  const { frames, activeFrameId, activeFrame, selectFrame, isSp00kyAvailable } = useDevTools();

  const [open, setOpen] = createSignal(false);
  let rootEl: HTMLDivElement | undefined;

  // The main document is always offered, even before it announces itself:
  // it is the default target, and "no client here" is a legitimate answer the
  // panel already renders via the status dot.
  const options = createMemo<Sp00kyFrame[]>(() => {
    const detected = frames();
    if (detected.some((f) => f.frameId === 0)) return detected;
    return [{ frameId: 0, url: '' }, ...detected];
  });

  const hasChoice = () => options().length > 1;

  const label = () => (activeFrameId() === 0 ? 'Main' : frameLabel(activeFrame()));

  onMount(() => {
    const onDocClick = (e: MouseEvent) => {
      if (open() && rootEl && !rootEl.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('click', onDocClick);
    document.addEventListener('keydown', onKey);
    onCleanup(() => {
      document.removeEventListener('click', onDocClick);
      document.removeEventListener('keydown', onKey);
    });
  });

  const pick = (frameId: number) => {
    selectFrame(frameId);
    setOpen(false);
  };

  const title = () => {
    const n = options().length;
    const where = activeFrameId() === 0 ? 'the main document' : frameTitle(activeFrame());
    return `${isSp00kyAvailable() ? 'Connected to' : 'No client detected in'} ${where}${
      n > 1 ? ` — ${n} clients in this tab, click to switch` : ''
    }`;
  };

  return (
    <div class="frame-select" ref={rootEl}>
      <button
        class="frame-select-btn"
        classList={{ open: open(), 'has-choice': hasChoice() }}
        title={title()}
        aria-label={title()}
        aria-expanded={open()}
        aria-haspopup="menu"
        onClick={() => setOpen((v) => !v)}
      >
        <span
          class="status-dot"
          classList={{ active: isSp00kyAvailable(), inactive: !isSp00kyAvailable() }}
        />
        {/* The label only earns toolbar width once there is something to choose
            between; the caret is always there, or nothing says the dot opens a
            menu. */}
        <Show when={hasChoice()}>
          <span class="frame-select-label">{label()}</span>
        </Show>
        <span class="frame-select-caret" aria-hidden="true">
          ▾
        </span>
      </button>

      <Show when={open()}>
        <div class="frame-select-menu" role="menu">
          <div class="frame-select-menu-head">Sp00ky clients in this tab</div>
          <For each={options()}>
            {(frame) => (
              <button
                class="frame-select-item"
                classList={{ active: frame.frameId === activeFrameId() }}
                role="menuitem"
                title={frameTitle(frame)}
                onClick={() => pick(frame.frameId)}
              >
                <span class="frame-select-item-main">
                  {frame.frameId === 0 ? 'Main document' : frameLabel(frame)}
                </span>
                <span class="frame-select-item-sub mono">
                  {frame.frameId === 0 ? hostOf(frame.url) : `iframe · ${hostOf(frame.url)}`}
                  <Show when={frame.version}>{(v) => <> · v{v()}</>}</Show>
                </span>
              </button>
            )}
          </For>
          {/* Chrome's inspectedWindow.eval addresses a frame by URL, so two
              iframes with the SAME url are one entry as far as evaluation is
              concerned. Say so rather than letting it look like a bug. */}
          <Show when={hasDuplicateUrls(options())}>
            <div class="frame-select-note">
              Two frames share a URL — queries run against whichever Chrome matches first.
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/** Last path segment (or host) — enough to tell frames apart in a narrow toolbar. */
function frameLabel(frame?: Sp00kyFrame): string {
  if (!frame) return 'Unknown';
  if (frame.frameId === 0) return 'Main';
  try {
    const url = new URL(frame.url);
    const last = url.pathname.split('/').filter(Boolean).pop();
    return last || url.host;
  } catch {
    return frame.url || `Frame ${frame.frameId}`;
  }
}

function hostOf(url: string): string {
  if (!url) return 'not detected yet';
  try {
    const u = new URL(url);
    return `${u.host}${u.pathname}`;
  } catch {
    return url;
  }
}

function frameTitle(frame?: Sp00kyFrame): string {
  if (!frame) return 'Unknown frame';
  return frame.frameId === 0 ? `main document (${frame.url})` : `iframe ${frame.url}`;
}

function hasDuplicateUrls(list: Sp00kyFrame[]): boolean {
  const seen = new Set<string>();
  for (const f of list) {
    if (f.frameId === 0 || !f.url) continue;
    if (seen.has(f.url)) return true;
    seen.add(f.url);
  }
  return false;
}
