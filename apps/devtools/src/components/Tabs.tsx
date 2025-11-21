import { For } from "solid-js";
import { useDevTools } from "../context/DevToolsContext";
import type { TabType } from "../types/devtools";

const tabs: { id: TabType; label: string }[] = [
  { id: "events", label: "Events" },
  { id: "queries", label: "Queries" },
  { id: "database", label: "Database" },
  { id: "auth", label: "Auth" },
];

export function Tabs() {
  const { activeTab, setActiveTab } = useDevTools();

  return (
    <div class="tabs">
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
    </div>
  );
}
