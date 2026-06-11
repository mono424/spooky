import { For, Show, createMemo, createSignal } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatMs } from '../../utils/formatters';
import type { QueryTimings } from '../../types/devtools';

// Compact cross-query columns (each shows the phase's p90). Click a header to
// sort by it; `total` (default) sorts by the sum of these p90s.
const COLS = [
  { key: 'ssp', label: 'SSP' },
  { key: 'localFetch', label: 'Local' },
  { key: 'remoteFetch', label: 'Remote' },
  { key: 'frontend', label: 'Frontend' },
] as const;

type SortKey = (typeof COLS)[number]['key'] | 'total';

function phaseP90(t: QueryTimings | undefined, key: keyof QueryTimings): number {
  const stat = t?.[key] as { p90?: number | null; lastMs?: number | null } | undefined;
  if (!stat || typeof stat !== 'object') return 0;
  return stat.p90 ?? stat.lastMs ?? 0;
}

function score(t: QueryTimings | undefined, key: SortKey): number {
  if (key === 'total') return COLS.reduce((acc, c) => acc + phaseP90(t, c.key), 0);
  return phaseP90(t, key);
}

/** Sortable, slowest-first table of all active queries × phase — for spotting
 *  the offending query/phase at a glance. */
export function TimingTab() {
  const { state, setActiveTab, setSelectedQueryHash } = useDevTools();
  const [sortKey, setSortKey] = createSignal<SortKey>('total');

  // Clicking a row's #hash jumps to the Queries tab with that query selected
  // (QueriesTab scrolls it into view + opens its detail panel).
  const openInQueries = (queryHash: number) => {
    setSelectedQueryHash(queryHash);
    setActiveTab('queries');
  };

  const rows = createMemo(() =>
    state.activeQueries
      .filter((q) => q.timings)
      .toSorted((a, b) => score(b.timings, sortKey()) - score(a.timings, sortKey()))
  );
  const maxTotal = createMemo(() =>
    Math.max(1e-6, ...rows().map((q) => score(q.timings, 'total')))
  );

  return (
    <div class="tt">
      <div class="tt-bar">
        <h2 class="tt-title">Query timings</h2>
        <span class="tt-hint">slowest first · click a column to sort</span>
      </div>

      <Show
        when={rows().length > 0}
        fallback={<div class="empty-state">No active queries with timing data yet</div>}
      >
        <div class="tt-table" role="table">
          <div class="tt-row tt-row--head" role="row">
            <span class="tt-q">Query</span>
            <For each={COLS}>
              {(c) => (
                <span
                  class="tt-num tt-sortable"
                  classList={{ active: sortKey() === c.key }}
                  onClick={() => setSortKey(c.key)}
                >
                  {c.label}
                </span>
              )}
            </For>
            <span
              class="tt-total tt-sortable"
              classList={{ active: sortKey() === 'total' }}
              onClick={() => setSortKey('total')}
            >
              Total
            </span>
            <span class="tt-meta">upd</span>
            <span class="tt-meta">err</span>
          </div>

          <For each={rows()}>
            {(q) => {
              const total = () => score(q.timings, 'total');
              const pct = () => `${Math.min(100, (total() / maxTotal()) * 100).toFixed(1)}%`;
              return (
                <div class="tt-row" role="row">
                  <span class="tt-q">
                    <span
                      class="tt-hash tt-hash--link"
                      role="link"
                      tabindex="0"
                      title="Open in Queries"
                      onClick={() => openInQueries(q.queryHash)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault();
                          openInQueries(q.queryHash);
                        }
                      }}
                    >
                      #{q.queryHash}
                    </span>
                    <span class="tt-sql" title={q.query}>{q.query}</span>
                  </span>
                  <For each={COLS}>
                    {(c) => (
                      <span class="tt-num" classList={{ active: sortKey() === c.key }}>
                        {formatMs(phaseP90(q.timings, c.key))}
                      </span>
                    )}
                  </For>
                  <span class="tt-total">
                    <span class="tt-bar-track">
                      <span class="tt-bar-fill" style={{ width: pct() }} />
                    </span>
                    <span class="tt-total-v">{formatMs(total())}</span>
                  </span>
                  {/* oxlint-disable-next-line no-non-null-assertion -- rows() filters to truthy timings */}
                  <span class="tt-meta">{q.timings!.updateCount}</span>
                  {/* oxlint-disable-next-line no-non-null-assertion -- rows() filters to truthy timings */}
                  <span class="tt-meta" classList={{ 'tt-err': q.timings!.errorCount > 0 }}>
                    {q.timings!.errorCount}
                  </span>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
