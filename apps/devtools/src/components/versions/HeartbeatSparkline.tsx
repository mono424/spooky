import { For, Show } from 'solid-js';
import { formatMs, formatRelativeTime } from '../../utils/formatters';
import type { HeartbeatInfo, HeartbeatSample } from '../../types/devtools';

/**
 * One bar per e2e heartbeat cycle, oldest → newest, height scaled to the
 * window's slowest successful sample. Failed cycles render as a full-height
 * error bar (they have no latency to plot, and a missing bar would read as
 * "quiet" rather than "broken").
 *
 * Hand-rolled CSS bars on purpose: the panel ships no charting library, and
 * this matches the `.tb-bar` idiom the timing views already use.
 *
 * The window is whatever the scheduler had in memory when `/info` was last
 * fetched — the panel has no poller, so it advances on Refresh (or when the
 * Versions tab's scoped refresh runs), not on its own.
 */
export function HeartbeatSparkline(props: { heartbeat: HeartbeatInfo }) {
  const samples = (): HeartbeatSample[] => props.heartbeat.samples ?? [];

  // Scale off successful samples only; a failure contributes no latency.
  const peak = (): number => {
    const oks = samples()
      .map((s) => s.ms)
      .filter((ms): ms is number => typeof ms === 'number');
    return oks.length > 0 ? Math.max(...oks) : 0;
  };

  const heightPct = (s: HeartbeatSample): string => {
    if (!s.ok || typeof s.ms !== 'number') return '100%';
    const max = peak();
    if (max <= 0) return '8%';
    // Floor at 8% so a fast sample is still visible as a mark.
    return `${Math.max(8, (s.ms / max) * 100).toFixed(1)}%`;
  };

  const title = (s: HeartbeatSample): string =>
    s.ok
      ? `${formatMs(s.ms)} · ${formatRelativeTime(s.ts)}`
      : `failed · ${formatRelativeTime(s.ts)}`;

  return (
    <Show when={samples().length > 0}>
      <div
        class="hb-spark"
        title={`e2e sync latency, last ${samples().length} probes (every ${props.heartbeat.interval_secs}s)`}
      >
        <For each={samples()}>
          {(s) => (
            <span
              class="hb-spark-bar"
              classList={{ failed: !s.ok }}
              style={{ height: heightPct(s) }}
              title={title(s)}
            />
          )}
        </For>
      </div>
    </Show>
  );
}
