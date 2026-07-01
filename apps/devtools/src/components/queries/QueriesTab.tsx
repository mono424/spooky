import { For, Show, createMemo, createEffect, createSignal } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatTime, formatRelativeTime, formatBytes } from '../../utils/formatters';
import { TimingBreakdown } from '../timing/TimingBreakdown';

function QueryList() {
  const { state, selectedQueryHash, setSelectedQueryHash } = useDevTools();
  const [filter, setFilter] = createSignal('');
  let listEl: HTMLDivElement | undefined;

  const matches = (q: { query?: string; queryHash: number }): boolean => {
    const term = filter().trim().toLowerCase();
    if (!term) return true;
    return (q.query ?? '').toLowerCase().includes(term) || String(q.queryHash).includes(term);
  };

  // Sort queries by createdAt in descending order (newest first)
  const sortedQueries = createMemo(() => {
    return state.activeQueries.filter(matches).toSorted((a, b) => b.createdAt - a.createdAt);
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
      <div class="queries-filter">
        <input
          class="dt-filter-input"
          type="text"
          placeholder="Filter queries…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
      </div>
      <div class="queries-list-content" ref={listEl}>
        <Show
          when={sortedQueries().length > 0}
          fallback={
            <div class="empty-state">
              {filter().trim() ? 'No queries match the filter' : 'No active queries'}
            </div>
          }
        >
          <For each={sortedQueries()}>
            {(query) => (
              <div
                class="query-item"
                data-query-hash={query.queryHash}
                classList={{ selected: selectedQueryHash() === query.queryHash }}
                onClick={() => setSelectedQueryHash(query.queryHash)}
                title={query.query}
              >
                <span class="query-preview">{query.query || '—'}</span>
                <span class="query-meta">
                  <span title="updates">{query.updateCount}</span>
                  <span class="query-meta-sep">·</span>
                  <span title="data size">{formatBytes(query.dataSize)}</span>
                </span>
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}

type DetailTab = 'overview' | 'query' | 'variables' | 'data' | 'timing';

const DETAIL_LABELS: Record<DetailTab, string> = {
  overview: 'Overview',
  query: 'Query',
  variables: 'Variables',
  data: 'Data',
  timing: 'Timing',
};

function QueryDetail() {
  const { state, selectedQueryHash } = useDevTools();
  const [tab, setTab] = createSignal<DetailTab>('overview');

  const selectedQuery = createMemo(() => {
    const hash = selectedQueryHash();
    if (hash === null) return null;
    return state.activeQueries.find((q) => q.queryHash === hash) ?? null;
  });

  // Which detail tabs have content for the current query (absent → hidden).
  const available = createMemo<DetailTab[]>(() => {
    const q = selectedQuery();
    if (!q) return ['overview'];
    const t: DetailTab[] = ['overview'];
    if (q.query) t.push('query');
    if (q.variables !== undefined && q.variables !== null) t.push('variables');
    if (q.data !== undefined || q.localArray !== undefined || q.remoteArray !== undefined) {
      t.push('data');
    }
    if (q.timings) t.push('timing');
    return t;
  });

  // Reset to Overview whenever the selected query changes.
  createEffect(() => {
    selectedQueryHash();
    setTab('overview');
  });

  const active = createMemo<DetailTab>(() => (available().includes(tab()) ? tab() : 'overview'));

  return (
    <div class="query-detail">
      <Show when={selectedQuery()} fallback={<div class="detail-empty">Select a query to inspect</div>}>
        {(query) => (
          <>
            <div class="detail-header">
              <h3 class="detail-query" title={query().query}>
                {query().query || `Query #${query().queryHash}`}
              </h3>
              <span class="detail-hash">#{query().queryHash}</span>
            </div>

            <div class="detail-tabs" role="tablist">
              <For each={available()}>
                {(t) => (
                  <button
                    class="detail-tab"
                    classList={{ active: active() === t }}
                    role="tab"
                    onClick={() => setTab(t)}
                  >
                    {DETAIL_LABELS[t]}
                  </button>
                )}
              </For>
            </div>

            <div class="detail-body">
              <Show when={active() === 'overview'}>
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
                  <Show when={query().ttl}>
                    <div class="kv-row">
                      <span class="kv-k">TTL</span>
                      <span class="kv-v mono">{query().ttl}</span>
                    </div>
                  </Show>
                  <Show when={query().dataSize !== undefined}>
                    <div class="kv-row">
                      <span class="kv-k">Data size</span>
                      <span class="kv-v mono">{formatBytes(query().dataSize)}</span>
                    </div>
                  </Show>
                </div>
              </Show>

              <Show when={active() === 'query'}>
                <pre class="code-block">{query().query}</pre>
              </Show>

              <Show when={active() === 'variables'}>
                <pre class="code-block">{JSON.stringify(query().variables, null, 2)}</pre>
              </Show>

              <Show when={active() === 'data'}>
                <Show when={query().data !== undefined}>
                  <section class="detail-block">
                    <h4 class="detail-block-title">Result data</h4>
                    <pre class="code-block">{JSON.stringify(query().data, null, 2)}</pre>
                  </section>
                </Show>
                <Show when={query().localArray !== undefined}>
                  <section class="detail-block">
                    <h4 class="detail-block-title">Local array</h4>
                    <pre class="code-block">{JSON.stringify(query().localArray, null, 2)}</pre>
                  </section>
                </Show>
                <Show when={query().remoteArray !== undefined}>
                  <section class="detail-block">
                    <h4 class="detail-block-title">Remote array</h4>
                    <pre class="code-block">{JSON.stringify(query().remoteArray, null, 2)}</pre>
                  </section>
                </Show>
              </Show>

              <Show when={active() === 'timing'}>
                {/* oxlint-disable-next-line no-non-null-assertion -- 'timing' only available when timings set */}
                <TimingBreakdown timings={query().timings!} />
              </Show>
            </div>
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
