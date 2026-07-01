import { For, createMemo } from 'solid-js';
import type { QueryTimings, PhaseStat } from '../../types/devtools';
import { formatMs } from '../../utils/formatters';

// Per-update phases in pipeline order. `ssp` is the whole WASM-ingest wall time;
// the three indented rows are its internal breakdown.
const PHASES: { key: keyof QueryTimings; label: string; sub?: boolean }[] = [
  { key: 'ssp', label: 'SSP · ingest' },
  { key: 'sspStoreApply', label: 'store apply', sub: true },
  { key: 'sspCircuitStep', label: 'circuit step', sub: true },
  { key: 'sspTransform', label: 'transform', sub: true },
  { key: 'localFetch', label: 'local fetch' },
  { key: 'remoteFetch', label: 'remote fetch' },
  { key: 'frontend', label: 'frontend reconcile' },
];

const p90Of = (s: PhaseStat | undefined) => s?.p90 ?? s?.lastMs ?? 0;

/** Detailed per-phase timing for a single query (rendered inside QueryDetail). */
export function TimingBreakdown(props: { timings: QueryTimings }) {
  const t = () => props.timings;
  const rows = createMemo(() => PHASES.map((p) => ({ ...p, stat: t()[p.key] as PhaseStat })));
  // Scale bars against the busiest top-level phase (exclude SSP sub-rows so the
  // sub-breakdown reads relative to the whole ingest, not itself).
  const maxP90 = createMemo(() =>
    Math.max(1e-6, ...rows().filter((r) => !r.sub).map((r) => p90Of(r.stat)))
  );

  const reg = () => t().registration;

  return (
    <div class="tb">
      <div class="tb-row tb-row--head">
        <span class="tb-phase">Phase</span>
        <span class="tb-num">last</span>
        <span class="tb-num">p50</span>
        <span class="tb-num tb-num--key">p90</span>
        <span class="tb-num">p99</span>
        <span class="tb-n">n</span>
      </div>

      <For each={rows()}>
        {(r) => {
          const pct = () => `${Math.min(100, (p90Of(r.stat) / maxP90()) * 100).toFixed(1)}%`;
          return (
            <div class="tb-row" classList={{ 'tb-row--sub': !!r.sub }}>
              <span class="tb-phase">
                <span class="tb-label">{r.label}</span>
                <span class="tb-bar">
                  <span class="tb-bar-fill" style={{ width: pct() }} />
                </span>
              </span>
              <span class="tb-num">{formatMs(r.stat?.lastMs)}</span>
              <span class="tb-num">{formatMs(r.stat?.p50)}</span>
              <span class="tb-num tb-num--key">{formatMs(r.stat?.p90)}</span>
              <span class="tb-num">{formatMs(r.stat?.p99)}</span>
              <span class="tb-n">{r.stat?.count ?? 0}</span>
            </div>
          );
        }}
      </For>

      <div class="tb-meta">
        <div class="tb-stat">
          <span class="tb-stat-label">registration</span>
          <div class="tb-stat-items">
            <For
              each={
                [
                  ['parse', reg().parseMs],
                  ['plan', reg().planMs],
                  ['snapshot', reg().snapshotMs],
                  ['wall', reg().wallMs],
                ] as const
              }
            >
              {([k, ms]) => (
                <span class="tb-stat-item">
                  {k}
                  <b>{formatMs(ms)}</b>
                </span>
              )}
            </For>
          </div>
        </div>
        <div class="tb-stat">
          <span class="tb-stat-label">totals</span>
          <div class="tb-stat-items">
            <span class="tb-stat-item">
              updates
              <b>{t().updateCount}</b>
            </span>
            <span class="tb-stat-item" classList={{ 'tb-stat-item--err': t().errorCount > 0 }}>
              errors
              <b>{t().errorCount}</b>
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
