import { For, Show, createSignal, createMemo, onMount, onCleanup } from 'solid-js';
import { useDevTools } from '../context/DevToolsContext';
import type { TabType } from '../types/devtools';

// Events moved to the end (was first); Queries is now the default landing tab.
const tabs: { id: TabType; label: string }[] = [
  { id: 'queries', label: 'Queries' },
  { id: 'timing', label: 'Timing' },
  { id: 'database', label: 'Database' },
  { id: 'storage', label: 'Storage' },
  { id: 'auth', label: 'Auth' },
  { id: 'flags', label: 'Flags' },
  { id: 'versions', label: 'Versions' },
  { id: 'mcp', label: 'MCP' },
  { id: 'events', label: 'Events' },
];

// Reserve for the » overflow button when it has to be shown.
const CHEVRON_W = 30;

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true">
      <path d="M17.65 6.35A7.958 7.958 0 0 0 12 4a8 8 0 1 0 7.75 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z" />
    </svg>
  );
}

function ClearIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true">
      <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zM4 12c0-4.42 3.58-8 8-8 1.85 0 3.55.63 4.9 1.69L5.69 16.9A7.902 7.902 0 0 1 4 12zm8 8c-1.85 0-3.55-.63-4.9-1.69L18.31 7.1A7.902 7.902 0 0 1 20 12c0 4.42-3.58 8-8 8z" />
    </svg>
  );
}

function DoubleChevronIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true">
      <path d="M6.41 6 5 7.41 9.58 12 5 16.59 6.41 18l6-6z" />
      <path d="m13 6-1.41 1.41L16.17 12l-4.58 4.59L13 18l6-6z" />
    </svg>
  );
}

export function Tabs() {
  const { activeTab, setActiveTab, isSp00kyAvailable, refresh, clearEvents } = useDevTools();

  let tabsEl: HTMLDivElement | undefined;
  let statusEl: HTMLDivElement | undefined;
  let actionsEl: HTMLDivElement | undefined;
  let overflowEl: HTMLDivElement | undefined;
  const tabRefs: Partial<Record<TabType, HTMLButtonElement>> = {};

  const [visibleCount, setVisibleCount] = createSignal(tabs.length);
  const [menuOpen, setMenuOpen] = createSignal(false);

  // Tab labels are static, so their intrinsic widths only need measuring once
  // (while every tab is still mounted on first paint).
  const tabWidths: Partial<Record<TabType, number>> = {};
  let measured = false;

  const measure = (): boolean => {
    if (measured) return true;
    for (const t of tabs) {
      const el = tabRefs[t.id];
      if (!el) return false;
      tabWidths[t.id] = el.offsetWidth;
    }
    measured = true;
    return true;
  };

  const recompute = () => {
    if (!tabsEl || !measure()) return;
    const containerW = tabsEl.clientWidth;
    const reserved = (statusEl?.offsetWidth ?? 0) + (actionsEl?.offsetWidth ?? 0) + 6;
    const availAll = containerW - reserved;
    const total = tabs.reduce((s, t) => s + (tabWidths[t.id] ?? 0), 0);
    if (total <= availAll) {
      setVisibleCount(tabs.length);
      return;
    }
    const avail = availAll - CHEVRON_W;
    let sum = 0;
    let n = 0;
    for (const t of tabs) {
      const w = tabWidths[t.id] ?? 0;
      if (sum + w <= avail) {
        sum += w;
        n++;
      } else break;
    }
    setVisibleCount(n);
  };

  // Split into visible + overflow, always keeping the active tab visible (pull
  // it out of the overflow menu, Chrome-style, displacing the last visible tab).
  const split = createMemo(() => {
    const n = visibleCount();
    let visible = tabs.slice(0, n);
    let overflow = tabs.slice(n);
    const act = activeTab();
    if (overflow.some((t) => t.id === act)) {
      // oxlint-disable-next-line no-non-null-assertion -- guarded by .some() above
      const actTab = overflow.find((t) => t.id === act)!;
      const rest = overflow.filter((t) => t.id !== act);
      if (visible.length > 0) {
        const displaced = visible[visible.length - 1];
        visible = [...visible.slice(0, -1), actTab];
        overflow = [displaced, ...rest];
      } else {
        visible = [actTab];
        overflow = rest;
      }
    }
    return { visible, overflow };
  });

  onMount(() => {
    requestAnimationFrame(recompute);

    const ro = new ResizeObserver(() => recompute());
    if (tabsEl) ro.observe(tabsEl);

    const onDocClick = (e: MouseEvent) => {
      if (menuOpen() && overflowEl && !overflowEl.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false);
    };
    document.addEventListener('click', onDocClick);
    document.addEventListener('keydown', onKey);

    onCleanup(() => {
      ro.disconnect();
      document.removeEventListener('click', onDocClick);
      document.removeEventListener('keydown', onKey);
    });
  });

  const pickOverflow = (id: TabType) => {
    setActiveTab(id);
    setMenuOpen(false);
  };

  return (
    <div class="tabs" ref={tabsEl}>
      <div class="toolbar-group" ref={statusEl}>
        <div class="status-indicator">
          <Show
            when={isSp00kyAvailable()}
            fallback={<span class="status-dot inactive" />}
          >
            <span class="status-dot active" />
          </Show>
        </div>
      </div>

      <For each={split().visible}>
        {(tab) => (
          <button
            class="tab-btn"
            ref={(el) => (tabRefs[tab.id] = el)}
            classList={{ active: activeTab() === tab.id }}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </button>
        )}
      </For>

      <Show when={split().overflow.length > 0}>
        <div class="tab-overflow" ref={overflowEl}>
          <button
            class="tab-overflow-btn"
            classList={{ open: menuOpen() }}
            title="More tabs"
            aria-label="More tabs"
            aria-expanded={menuOpen()}
            onClick={() => setMenuOpen((v) => !v)}
          >
            <DoubleChevronIcon />
          </button>
          <Show when={menuOpen()}>
            <div class="tab-overflow-menu" role="menu">
              <For each={split().overflow}>
                {(tab) => (
                  <button
                    class="tab-overflow-item"
                    classList={{ active: activeTab() === tab.id }}
                    role="menuitem"
                    onClick={() => pickOverflow(tab.id)}
                  >
                    {tab.label}
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>

      <div class="toolbar-group-right" ref={actionsEl}>
        <button class="icon-btn" title="Refresh" aria-label="Refresh" onClick={refresh}>
          <RefreshIcon />
        </button>
        <button
          class="icon-btn"
          title="Clear events"
          aria-label="Clear events"
          onClick={clearEvents}
        >
          <ClearIcon />
        </button>
      </div>
    </div>
  );
}
