import { For, Show, createSignal, createMemo } from "solid-js";
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
  const [selectedTypes, setSelectedTypes] = createSignal<Set<string>>(new Set());

  const availableTypes = createMemo(() => {
    const types = new Set<string>();
    state.events.forEach((e) => types.add(e.type));
    return Array.from(types).sort();
  });

  const filteredEvents = createMemo(() => {
    const selected = selectedTypes();
    if (selected.size === 0) return state.events;
    return state.events.filter((e) => selected.has(e.type));
  });

  const toggleType = (type: string) => {
    setSelectedTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) {
        next.delete(type);
      } else {
        next.add(type);
      }
      return next;
    });
  };

  return (
    <div class="events-container">
      <div class="events-header">
        <h2>Events History</h2>
      </div>

      <Show when={availableTypes().length > 0}>
        <div class="events-filter-bar">
          <For each={availableTypes()}>
            {(type) => (
              <button
                class={`filter-chip ${selectedTypes().has(type) ? "active" : ""}`}
                onClick={() => toggleType(type)}
              >
                {type}
              </button>
            )}
          </For>
        </div>
      </Show>

      <div class="events-list">
        <Show
          when={state.events.length > 0}
          fallback={<div class="empty-state">No events recorded yet</div>}
        >
          <For each={filteredEvents()}>
            {(event) => <EventItem event={event} />}
          </For>
        </Show>
      </div>
    </div>
  );
}
