import { For, Show } from "solid-js";
import { useDevTools } from "../context/DevToolsContext";
import type { TabType } from "../types/devtools";

const tabs: { id: TabType; label: string }[] = [
  { id: "events", label: "Events" },
  { id: "queries", label: "Queries" },
  { id: "database", label: "Database" },
  { id: "auth", label: "Auth" },
];

export function Tabs() {
  const { activeTab, setActiveTab, isSpookyAvailable, refresh, clearEvents } =
    useDevTools();

  return (
    <div class="tabs">
      <div class="toolbar-group">
        <div class="status-indicator">
          <Show
            when={isSpookyAvailable()}
            fallback={
              <>
                <span class="status-dot inactive" />
              </>
            }
          >
            <span class="status-dot active" />
          </Show>
        </div>
      </div>
      <For each={tabs}>
        {(tab) => (
          <button
            class="tab-btn"
            classList={{ active: activeTab() === tab.id }}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </button>
        )}
      </For>
      <div class="toolbar-group-right">
        <button class="btn" onClick={refresh}>
          Refresh
        </button>
        <button class="btn" onClick={clearEvents}>
          Clear Events
        </button>
      </div>
    </div>
  );
}
