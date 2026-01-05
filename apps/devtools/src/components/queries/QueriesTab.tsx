import { For, Show, createMemo } from "solid-js";
import { useDevTools } from "../../context/DevToolsContext";
import {
  formatTime,
  formatRelativeTime,
  formatBytes,
} from "../../utils/formatters";
import { QueryGraph } from "./QueryGraph";

function QueryList() {
  const { state, selectedQueryHash, setSelectedQueryHash } = useDevTools();

  // Sort queries by createdAt in descending order (newest first)
  const sortedQueries = createMemo(() => {
    return [...state.activeQueries].sort((a, b) => b.createdAt - a.createdAt);
  });

  return (
    <div class="queries-list">
      <div class="queries-header">
        <h2>Active Queries</h2>
      </div>
      <div class="queries-list-content">
        <Show
          when={sortedQueries().length > 0}
          fallback={<div class="empty-state">No active queries</div>}
        >
          <For each={sortedQueries()}>
            {(query) => (
              <div
                class="query-item"
                classList={{ selected: selectedQueryHash() === query.queryHash }}
                onClick={() => setSelectedQueryHash(query.queryHash)}
              >
                <div class="query-header">
                  <span class="query-hash">#{query.queryHash}</span>
                  <span class={`query-status status-${query.status}`}>
                    {query.status}
                  </span>
                </div>
                <div class="query-meta">
                  Updates: {query.updateCount} | Size:{" "}
                  {formatBytes(query.dataSize)}
                </div>
                <Show when={query.query}>
                  <div class="query-preview">
                    {query.query!.substring(0, 50)}
                    {query.query!.length > 50 ? "..." : ""}
                  </div>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}

function QueryDetail() {
  const { state, selectedQueryHash } = useDevTools();

  const selectedQuery = createMemo(() => {
    const hash = selectedQueryHash();
    if (hash === null) return null;
    return state.activeQueries.find((q) => q.queryHash === hash);
  });

  return (
    <div class="query-detail">
      <Show when={selectedQuery()}>
        {(query) => (
          <>
            <div class="detail-header">
              <h3>Query #{query().queryHash}</h3>
              <span class={`query-status status-${query().status}`}>
                {query().status}
              </span>
            </div>

            <div class="detail-section">
              <div class="detail-label">Created</div>
              <div class="detail-value">
                {formatTime(query().createdAt)} (
                {formatRelativeTime(query().createdAt)})
              </div>
            </div>

            <div class="detail-section">
              <div class="detail-label">Last Update</div>
              <div class="detail-value">
                {formatTime(query().lastUpdate)} (
                {formatRelativeTime(query().lastUpdate)})
              </div>
            </div>

            <div class="detail-section">
              <div class="detail-label">Update Count</div>
              <div class="detail-value mono">{query().updateCount}</div>
            </div>

            <Show when={query().dataSize !== undefined}>
              <div class="detail-section">
                <div class="detail-label">Data Size</div>
                <div class="detail-value mono">
                  {formatBytes(query().dataSize)}
                </div>
              </div>
            </Show>

            <Show when={query().query}>
              <div class="detail-section">
                <div class="detail-label">Query</div>
                <pre class="query-code">{query().query}</pre>
              </div>
            </Show>

            <Show when={query().variables}>
              <div class="detail-section">
                <div class="detail-label">Variables</div>
                <pre class="query-code">
                  {JSON.stringify(query().variables, null, 2)}
                </pre>
              </div>
            </Show>

            <Show when={query().data}>
              <div class="detail-section">
                <div class="detail-label">Result Data</div>
                <pre class="query-code">
                  {JSON.stringify(query().data, null, 2)}
                </pre>
              </div>
            </Show>

            <QueryGraph query={query()} allQueries={state.activeQueries} />
          </>
        )}
      </Show>
    </div>
  );
}

export function QueriesTab() {
  return (
    <div class="queries-container">
      <QueryList />
      <QueryDetail />
    </div>
  );
}
