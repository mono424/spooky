import { For, Show } from "solid-js";
import { useDevTools } from "../../context/DevToolsContext";
import { formatTime } from "../../utils/formatters";
import type { SpookyEvent } from "../../types/devtools";

function EventItem(props: { event: SpookyEvent }) {
  return (
    <div class="event-item">
      <div class="event-header">
        <span class="event-type">{props.event.type}</span>
        <span class="event-time">{formatTime(props.event.timestamp)}</span>
      </div>
      <Show when={props.event.data}>
        <div class="event-payload">
          <pre>{JSON.stringify(props.event.data, null, 2)}</pre>
        </div>
      </Show>
    </div>
  );
}

export function EventsTab() {
  const { state } = useDevTools();

  return (
    <div class="events-container">
      <div class="events-header">
        <h2>Events History</h2>
      </div>
      <div class="events-list">
        <Show
          when={state.events.length > 0}
          fallback={<div class="empty-state">No events recorded yet</div>}
        >
          <For each={state.events}>
            {(event) => <EventItem event={event} />}
          </For>
        </Show>
      </div>
    </div>
  );
}
