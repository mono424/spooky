import {
  For,
  Show,
  createMemo,
  createEffect,
  createSignal,
  type Accessor,
  type JSX,
} from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatTime, formatRelativeTime, formatBytes } from '../../utils/formatters';
import { TimingBreakdown } from '../timing/TimingBreakdown';
import { QueryTimeline } from './QueryTimeline';
import { JsonView } from '../ui/JsonView';
import type { ActiveQuery } from '../../types/devtools';

function QueryTable(props: { queries: Accessor<ActiveQuery[]>; filterActive: boolean }) {
  const { selectedQueryHash, setSelectedQueryHash } = useDevTools();
  let tableEl: HTMLDivElement | undefined;

  const collapsed = () => selectedQueryHash() !== null;

  // Bring the selected query into view whenever the selection changes — e.g.
  // when the user clicks a #hash on the Timing tab and we jump here.
  createEffect(() => {
    const hash = selectedQueryHash();
    if (hash === null || !tableEl) return;
    // Defer so the row is in the DOM (tab just switched / list just rendered).
    queueMicrotask(() => {
      tableEl
        ?.querySelector(`[data-query-hash="${hash}"]`)
        ?.scrollIntoView({ block: 'nearest' });
    });
  });

  return (
    <div class="qt-table-wrap" classList={{ collapsed: collapsed() }} ref={tableEl}>
      <Show
        when={props.queries().length > 0}
        fallback={
          <div class="empty-state">
            {props.filterActive ? 'No queries match the filter' : 'No active queries'}
          </div>
        }
      >
        <table class="qt-table">
          <thead>
            <tr>
              <th class="qt-col-query">Query</th>
              <Show when={!collapsed()}>
                <th>Status</th>
                <th class="qt-num">Updates</th>
                <th class="qt-num">Size</th>
                <th>Created</th>
                <th>Last update</th>
              </Show>
            </tr>
          </thead>
          <tbody>
            <For each={props.queries()}>
              {(query) => (
                <tr
                  data-query-hash={query.queryHash}
                  classList={{ selected: selectedQueryHash() === query.queryHash }}
                  onClick={() => setSelectedQueryHash(query.queryHash)}
                  title={query.query}
                >
                  <td class="qt-col-query">
                    <span class="qt-query-text">{query.query || `#${query.queryHash}`}</span>
                  </td>
                  <Show when={!collapsed()}>
                    <td>
                      <span class={`status-pill status-${query.status}`}>{query.status}</span>
                    </td>
                    <td class="qt-num">{query.updateCount}</td>
                    <td class="qt-num">{formatBytes(query.dataSize)}</td>
                    <td class="qt-time">{formatTime(query.createdAt)}</td>
                    <td class="qt-time">{formatRelativeTime(query.lastUpdate)}</td>
                  </Show>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </div>
  );
}

type DetailTab = 'overview' | 'query' | 'data' | 'timing';

const DETAIL_LABELS: Record<DetailTab, string> = {
  overview: 'Overview',
  query: 'Query',
  data: 'Data',
  timing: 'Timing',
};

/** Chrome-devtools style collapsible section ("▼ General"). */
function Section(props: { title: string; children: JSX.Element }) {
  return (
    <details class="detail-section" open>
      <summary class="detail-section-title">{props.title}</summary>
      <div class="detail-section-body">{props.children}</div>
    </details>
  );
}

function QueryDetail() {
  const { state, selectedQueryHash, setSelectedQueryHash } = useDevTools();
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
    if (q.query || (q.variables !== undefined && q.variables !== null)) t.push('query');
    if (q.data !== undefined || q.localArray !== undefined || q.remoteArray !== undefined) {
      t.push('data');
    }
    if (q.timings) t.push('timing');
    return t;
  });

  // The chosen tab sticks while switching queries (panel stays open); `active`
  // falls back to Overview only when the new query lacks that tab's content.
  // Closing the panel unmounts this component, so reopening starts at Overview.
  const active = createMemo<DetailTab>(() => (available().includes(tab()) ? tab() : 'overview'));

  return (
    <div class="query-detail">
      <Show when={selectedQuery()} fallback={<div class="detail-empty">Select a query to inspect</div>}>
        {(query) => (
          <>
            <div class="detail-tabs" role="tablist">
              <button
                class="detail-close"
                title="Close detail panel"
                onClick={() => setSelectedQueryHash(null)}
              >
                ✕
              </button>
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
                <Section title="General">
                  <div class="kv">
                    <div class="kv-row">
                      <span class="kv-k">Query</span>
                      <span class="kv-v mono kv-wrap">{query().query || '—'}</span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k">Hash</span>
                      <span class="kv-v mono">#{query().queryHash}</span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k">Status</span>
                      <span class="kv-v">
                        <span class={`status-pill status-${query().status}`}>{query().status}</span>
                      </span>
                    </div>
                    <Show when={query().ttl}>
                      <div class="kv-row">
                        <span class="kv-k">TTL</span>
                        <span class="kv-v mono">{query().ttl}</span>
                      </div>
                    </Show>
                  </div>
                </Section>
                <Section title="Activity">
                  <div class="kv">
                    <div class="kv-row">
                      <span class="kv-k">Created</span>
                      <span class="kv-v mono">
                        {formatTime(query().createdAt)}{' '}
                        <span class="kv-rel">{formatRelativeTime(query().createdAt)}</span>
                      </span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k">Last update</span>
                      <span class="kv-v mono">
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
                </Section>
              </Show>

              <Show when={active() === 'query'}>
                <Show when={query().query}>
                  <Section title="Query text">
                    <pre class="code-block">{query().query}</pre>
                  </Section>
                </Show>
                <Show when={query().variables !== undefined && query().variables !== null}>
                  <Section title="Variables">
                    <JsonView class="code-block" value={query().variables} />
                  </Section>
                </Show>
              </Show>

              <Show when={active() === 'data'}>
                <Show when={query().data !== undefined}>
                  <Section title="Result data">
                    <JsonView class="code-block" value={query().data} />
                  </Section>
                </Show>
                <Show when={query().localArray !== undefined}>
                  <Section title="Local array">
                    <JsonView class="code-block" value={query().localArray} />
                  </Section>
                </Show>
                <Show when={query().remoteArray !== undefined}>
                  <Section title="Remote array">
                    <JsonView class="code-block" value={query().remoteArray} />
                  </Section>
                </Show>
              </Show>

              <Show when={active() === 'timing'}>
                <Section title="Timing breakdown">
                  {/* oxlint-disable-next-line no-non-null-assertion -- 'timing' only available when timings set */}
                  <TimingBreakdown timings={query().timings!} />
                </Section>
              </Show>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}

function StatusBar(props: {
  visible: Accessor<ActiveQuery[]>;
  filter: Accessor<string>;
  setFilter: (value: string) => void;
}) {
  const { state } = useDevTools();

  const totalUpdates = createMemo(() =>
    state.activeQueries.reduce((sum, q) => sum + (q.updateCount || 0), 0)
  );
  const totalSize = createMemo(() =>
    state.activeQueries.reduce((sum, q) => sum + (q.dataSize || 0), 0)
  );
  const activeCount = createMemo(
    () =>
      state.activeQueries.filter((q) => q.status === 'active' || q.status === 'updating').length
  );

  return (
    <div class="queries-statusbar">
      <input
        class="dt-filter-input statusbar-filter"
        type="text"
        placeholder="Filter"
        value={props.filter()}
        onInput={(e) => props.setFilter(e.currentTarget.value)}
      />
      <span class="statusbar-sep" />
      <span>
        {props.filter().trim()
          ? `${props.visible().length} / ${state.activeQueries.length} queries`
          : `${state.activeQueries.length} queries`}
      </span>
      <span class="statusbar-sep" />
      <span>{totalUpdates()} updates</span>
      <span class="statusbar-sep" />
      <span>{formatBytes(totalSize())} data</span>
      <span class="statusbar-sep" />
      <span>{activeCount()} active</span>
    </div>
  );
}

export function QueriesTab() {
  const { state, selectedQueryHash } = useDevTools();
  const [filter, setFilter] = createSignal('');

  const matches = (q: ActiveQuery): boolean => {
    const term = filter().trim().toLowerCase();
    if (!term) return true;
    return (q.query ?? '').toLowerCase().includes(term) || String(q.queryHash).includes(term);
  };

  // Sort queries by createdAt in descending order (newest first)
  const sortedQueries = createMemo(() => {
    return state.activeQueries.filter(matches).toSorted((a, b) => b.createdAt - a.createdAt);
  });

  const filterActive = () => filter().trim().length > 0;

  return (
    <div class="queries-container">
      <QueryTimeline />
      <div class="queries-body">
        <QueryTable queries={sortedQueries} filterActive={filterActive()} />
        <Show when={selectedQueryHash() !== null}>
          <QueryDetail />
        </Show>
      </div>
      <StatusBar visible={sortedQueries} filter={filter} setFilter={setFilter} />
    </div>
  );
}
