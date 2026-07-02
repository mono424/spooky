import { For, Show, createMemo, createSignal, onCleanup } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatTime } from '../../utils/formatters';

/** Tick label without milliseconds — the strip is too dense for full precision. */
function formatTick(timestamp: number): string {
  const date = new Date(timestamp);
  const hours = date.getHours().toString().padStart(2, '0');
  const minutes = date.getMinutes().toString().padStart(2, '0');
  const seconds = date.getSeconds().toString().padStart(2, '0');
  return `${hours}:${minutes}:${seconds}`;
}

const TICK_COUNT = 8;

/**
 * Chrome network-tab style overview strip: one thin vertical bar per timeline
 * mark (query registered / query updated), on a wall-clock axis that keeps
 * extending while the panel is open.
 */
export function QueryTimeline() {
  const { state, queryMarks, selectedQueryHash, setSelectedQueryHash } = useDevTools();

  // 1s heartbeat so the axis end tracks "now" even when no marks arrive.
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));

  const domain = createMemo<{ start: number; end: number } | null>(() => {
    const marks = queryMarks();
    if (marks.length === 0) return null;
    // Marks arrive per snapshot in query order, not time order — scan for the
    // true bounds.
    let start = Infinity;
    let end = now();
    for (const m of marks) {
      if (m.timestamp < start) start = m.timestamp;
      if (m.timestamp > end) end = m.timestamp;
    }
    // Guard against a zero-width domain (single mark, first second).
    return { start, end: end - start < 1000 ? start + 1000 : end };
  });

  const ticks = createMemo(() => {
    const d = domain();
    if (!d) return [];
    return Array.from({ length: TICK_COUNT }, (_, i) => {
      const frac = (i + 1) / TICK_COUNT;
      return { left: frac * 100, time: d.start + frac * (d.end - d.start) };
    });
  });

  const queryText = (hash: number): string => {
    const q = state.activeQueries.find((entry) => entry.queryHash === hash);
    const text = q?.query ?? `#${hash}`;
    return text.length > 120 ? `${text.slice(0, 120)}…` : text;
  };

  return (
    <div class="query-timeline">
      <Show when={domain()} fallback={<div class="qtl-empty">No query activity yet</div>}>
        {(d) => (
          <>
            <For each={ticks()}>
              {(tick) => (
                <div class="qtl-tick" style={{ left: `${tick.left}%` }}>
                  <span class="qtl-tick-label">{formatTick(tick.time)}</span>
                </div>
              )}
            </For>
            <For each={queryMarks()}>
              {(mark) => (
                <div
                  class={`qtl-mark ${mark.kind}`}
                  classList={{ selected: selectedQueryHash() === mark.queryHash }}
                  style={{
                    left: `${((mark.timestamp - d().start) / (d().end - d().start)) * 100}%`,
                  }}
                  title={`${mark.kind === 'registered' ? 'Registered' : 'Updated'} ${formatTime(mark.timestamp)}\n${queryText(mark.queryHash)}`}
                  onClick={() => setSelectedQueryHash(mark.queryHash)}
                />
              )}
            </For>
          </>
        )}
      </Show>
    </div>
  );
}
