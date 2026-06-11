import { For, Show, createMemo, createEffect } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatTime, formatRelativeTime, formatBytes } from '../../utils/formatters';
import { TimingBreakdown } from '../timing/TimingBreakdown';

function QueryList() {
  const { state, selectedQueryHash, setSelectedQueryHash } = useDevTools();
  let listEl: HTMLDivElement | undefined;

  // Sort queries by createdAt in descending order (newest first)
  const sortedQueries = createMemo(() => {
    return [...state.activeQueries].toSorted((a, b) => b.createdAt - a.createdAt);
  });

  // Bring the selected query into view whenever the selection changes — e.g.
  // when the user clicks a #hash on the Timing tab and we jump here.
  createEffect(() => {
    const hash = selectedQueryHash();
    if (hash === null || !listEl) return;
    // Defer so the row is in the DOM (tab just switched / list just rendered).
    queueMicrotask(() => {
      listEl
        ?.querySelector(`[data-query-hash="${hash}"]`)
        ?.scrollIntoView({ block: 'nearest' });
    });
  });

  return (
    <div class="queries-list">
      <div class="queries-header">
        <h2>Active Queries</h2>
        <span class="queries-count">{sortedQueries().length}</span>
      </div>
      <div class="queries-list-content" ref={listEl}>
        <Show
          when={sortedQueries().length > 0}
          fallback={<div class="empty-state">No active queries</div>}
        >
          <For each={sortedQueries()}>
            {(query) => (
              <div
                class="query-item"
                data-query-hash={query.queryHash}
                classList={{ selected: selectedQueryHash() === query.queryHash }}
                onClick={() => setSelectedQueryHash(query.queryHash)}
              >
                <div class="query-item-top">
                  <span class="query-hash">#{query.queryHash}</span>
                  <span class={`status-pill status-${query.status}`}>
                    <span class="status-dot" />
                    {query.status}
                  </span>
                </div>
                <Show when={query.query}>
                  <div class="query-preview">{query.query}</div>
                </Show>
                <div class="query-meta">
                  <span>{query.updateCount} upd</span>
                  <span class="query-meta-sep">·</span>
                  <span>{formatBytes(query.dataSize)}</span>
                </div>
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
      <Show when={selectedQuery()} fallback={<div class="detail-empty">Select a query to inspect</div>}>
        {(query) => (
          <>
            <div class="detail-header">
              <h3>Query #{query().queryHash}</h3>
              <span class={`status-pill status-${query().status}`}>
                <span class="status-dot" />
                {query().status}
              </span>
            </div>

            <div class="kv">
              <div class="kv-row">
                <span class="kv-k">Created</span>
                <span class="kv-v">
                  {formatTime(query().createdAt)}{' '}
                  <span class="kv-rel">{formatRelativeTime(query().createdAt)}</span>
                </span>
              </div>
              <div class="kv-row">
                <span class="kv-k">Last update</span>
                <span class="kv-v">
                  {formatTime(query().lastUpdate)}{' '}
                  <span class="kv-rel">{formatRelativeTime(query().lastUpdate)}</span>
                </span>
              </div>
              <div class="kv-row">
                <span class="kv-k">Updates</span>
                <span class="kv-v mono">{query().updateCount}</span>
              </div>
              <Show when={query().dataSize !== undefined}>
                <div class="kv-row">
                  <span class="kv-k">Data size</span>
                  <span class="kv-v mono">{formatBytes(query().dataSize)}</span>
                </div>
              </Show>
            </div>

            <Show when={query().timings}>
              <section class="detail-block">
                <h4 class="detail-block-title">Timing</h4>
                {/* oxlint-disable-next-line no-non-null-assertion -- guarded by Show */}
                <TimingBreakdown timings={query().timings!} />
              </section>
            </Show>

            <Show when={query().query}>
              <section class="detail-block">
                <h4 class="detail-block-title">Query</h4>
                <pre class="code-block">{query().query}</pre>
              </section>
            </Show>

            <Show when={query().variables}>
              <section class="detail-block">
                <h4 class="detail-block-title">Variables</h4>
                <pre class="code-block">{JSON.stringify(query().variables, null, 2)}</pre>
              </section>
            </Show>

            <Show when={query().localArray}>
              <section class="detail-block">
                <h4 class="detail-block-title">Local array</h4>
                <pre class="code-block">{JSON.stringify(query().localArray, null, 2)}</pre>
              </section>
            </Show>

            <Show when={query().remoteArray}>
              <section class="detail-block">
                <h4 class="detail-block-title">Remote array</h4>
                <pre class="code-block">{JSON.stringify(query().remoteArray, null, 2)}</pre>
              </section>
            </Show>

            <Show when={query().data}>
              <section class="detail-block">
                <h4 class="detail-block-title">Result data</h4>
                <pre class="code-block">{JSON.stringify(query().data, null, 2)}</pre>
              </section>
            </Show>
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
