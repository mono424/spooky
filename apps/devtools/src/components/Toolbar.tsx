import { Show } from "solid-js";
import { useDevTools } from "../context/DevToolsContext";

export function Toolbar() {
  const { isSpookyAvailable, refresh, clearEvents } = useDevTools();

  return (
    <div class="toolbar">
      <div class="toolbar-group">
        <div class="status-indicator">
          <Show
            when={isSpookyAvailable()}
            fallback={
              <>
                <span class="status-dot inactive" />
                <span>Spooky not detected on this page</span>
              </>
            }
          >
            <span class="status-dot active" />
            <span>Spooky connected</span>
          </Show>
        </div>
      </div>
      <div class="toolbar-spacer" />
      <div class="toolbar-group">
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
