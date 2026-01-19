import { createSignal } from "solid-js";
import { TableList } from "./TableList";
import { TableView } from "./TableView";
import { Toast } from "../ui/Toast";

export function DatabaseTab() {
  const [filter, setFilter] = createSignal("");
  const [source, setSource] = createSignal<'local' | 'remote'>('local'); // Default to local
  const [error, setError] = createSignal<string | null>(null);

  const handleError = (msg: string) => {
    setError(msg);
  };

  return (
    <div style={{ display: "flex", "flex-direction": "column", height: "100%", width: "100%" }}>
        {error() && <Toast message={error()!} type="error" onDismiss={() => setError(null)} />}
        <div class="table-controls" style={{ height: "25px", padding: "0 8px", "border-bottom": "1px solid var(--sys-color-divider)", display: "flex", "align-items": "center", gap: "8px", "box-sizing": "border-box", "flex-shrink": 0, "background": "var(--sys-color-surface)" }}>
        <input
          type="text"
          placeholder="Filter..."
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
          style={{
            "flex": "1",
            "padding": "0 8px",
            "height": "18px",
            "background": "var(--sys-color-surface-container-highest, #3d3d3d)",
            "border": "1px solid var(--sys-color-outline-variant, #555)",
            "color": "var(--sys-color-on-surface, #fff)",
            "border-radius": "9px",
            "font-family": "var(--sys-typescale-body-font, '.SFNSDisplay-Regular', 'Helvetica Neue', 'Lucida Grande', sans-serif)",
            "font-size": "11px",
            "line-height": "14px",
            "outline": "none"
          }}
          onFocus={(e) => e.currentTarget.style.border = "1px solid var(--sys-color-primary, #1a73e8)"}
          onBlur={(e) => e.currentTarget.style.border = "1px solid var(--sys-color-outline-variant, #555)"}
        />
        <select
            value={source()}
            onChange={(e) => setSource(e.currentTarget.value as 'local' | 'remote')}
            style={{
                "height": "18px",
                "background": "transparent",
                "border": "1px solid var(--sys-color-outline-variant, #555)",
                "color": "var(--sys-color-on-surface, #fff)",
                "border-radius": "9px",
                "font-size": "11px",
                "padding": "0 8px",
                "outline": "none",
                "cursor": "pointer"
            }}
        >
            <option value="local">Local</option>
            <option value="remote">Remote</option>
        </select>
      </div>
      <div class="database-container">
        <TableList />
        <TableView filter={filter()} setFilter={setFilter} source={source()} onError={handleError} />
      </div>
    </div>
  );
}
