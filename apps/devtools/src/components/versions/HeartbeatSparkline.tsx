import { For, Show, createMemo, createUniqueId } from 'solid-js';
import { formatMs, formatRelativeTime } from '../../utils/formatters';
import type { HeartbeatInfo, HeartbeatSample } from '../../types/devtools';

/** Plot box in user units. Rendered with `preserveAspectRatio="none"`, so the
 *  chart stretches to whatever width the row gives it; strokes opt out of that
 *  scaling via `vector-effect` to stay crisp. */
const W = 240;
const H = 44;

/**
 * End-to-end sync latency over the scheduler's recent probe cycles.
 *
 * An area+line chart rather than the bar strip this replaces: with bars scaled
 * to the window peak, a steady pipeline (every sample within a millisecond of
 * the others) rendered as one solid block, and a failed cycle — which has no
 * latency at all — had to be drawn full-height, so an outage looked like the
 * slowest possible response instead of the absence of one.
 *
 * Here failures break the line (a gap is the honest shape for "no measurement")
 * and are marked on the baseline, while the y-axis runs from zero so bar height
 * stays proportional to actual latency.
 *
 * Hand-rolled SVG on purpose: the panel ships no charting library.
 */
export function HeartbeatSparkline(props: { heartbeat: HeartbeatInfo }) {
  const gradientId = createUniqueId();

  const samples = (): HeartbeatSample[] => props.heartbeat.samples ?? [];

  const stats = createMemo(() => {
    const all = samples();
    const oks = all.filter((s) => s.ok && typeof s.ms === 'number');
    const values = oks.map((s) => s.ms as number);
    return {
      count: all.length,
      failures: all.length - oks.length,
      max: values.length ? Math.max(...values) : 0,
      min: values.length ? Math.min(...values) : 0,
      avg: values.length ? values.reduce((a, b) => a + b, 0) / values.length : 0,
    };
  });

  // Headroom so the peak isn't drawn flush against the top edge. The floor
  // keeps a flat, healthy line sitting low in the box instead of filling it.
  const top = () => Math.max(stats().max * 1.25, 10);

  const xOf = (i: number): number => {
    const n = samples().length;
    return n <= 1 ? W / 2 : (i / (n - 1)) * W;
  };
  const yOf = (ms: number): number => H - Math.min(1, ms / top()) * H;

  /** Contiguous runs of successful samples — a failure ends the current run,
   *  so the line breaks rather than interpolating across a gap in knowledge. */
  const segments = createMemo(() => {
    const runs: { x: number; y: number }[][] = [];
    let run: { x: number; y: number }[] = [];
    samples().forEach((s, i) => {
      if (s.ok && typeof s.ms === 'number') {
        run.push({ x: xOf(i), y: yOf(s.ms) });
      } else if (run.length) {
        runs.push(run);
        run = [];
      }
    });
    if (run.length) runs.push(run);
    return runs;
  });

  const areaPath = (run: { x: number; y: number }[]): string => {
    if (run.length === 1) {
      // A lone point has no area; draw a hairline column so it still reads.
      return `M ${run[0]!.x} ${H} L ${run[0]!.x} ${run[0]!.y} L ${run[0]!.x} ${H} Z`;
    }
    const line = run.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');
    return `${line} L ${run[run.length - 1]!.x} ${H} L ${run[0]!.x} ${H} Z`;
  };

  const linePath = (run: { x: number; y: number }[]): string =>
    run.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');

  /** Contiguous failure runs as bands rather than one mark per sample: an
   *  outage is a stretch of time with no measurement, and a row of identical
   *  ticks reads as fifteen separate events instead of one ongoing one. */
  const failureBands = createMemo(() => {
    const bands: { from: number; to: number; count: number; ts: number }[] = [];
    let start: number | null = null;
    let count = 0;
    let ts = 0;
    samples().forEach((s, i) => {
      if (!s.ok) {
        if (start === null) {
          start = i;
          ts = s.ts;
        }
        count++;
      } else if (start !== null) {
        bands.push({ from: xOf(start), to: xOf(i), count, ts });
        start = null;
        count = 0;
      }
    });
    if (start !== null) {
      bands.push({ from: xOf(start), to: W, count, ts });
    }
    return bands;
  });

  const lastOk = createMemo(() => {
    const oks = samples().filter((s) => s.ok && typeof s.ms === 'number');
    return oks.length ? oks[oks.length - 1]! : null;
  });

  const degraded = () => props.heartbeat.stale || props.heartbeat.consecutive_failures > 0;

  const windowLabel = () => {
    const spanSecs = Math.max(1, stats().count - 1) * props.heartbeat.interval_secs;
    return spanSecs >= 3600
      ? `${(spanSecs / 3600).toFixed(1)}h`
      : `${Math.round(spanSecs / 60)}m`;
  };

  return (
    <div class="hb-chart" classList={{ degraded: degraded() }}>
      <div class="hb-chart-head">
        <span class="hb-chart-title">e2e sync latency</span>
        <span class="hb-chart-now">
          <Show when={props.heartbeat.last_e2e_ms !== null} fallback={<span class="muted">—</span>}>
            {formatMs(props.heartbeat.last_e2e_ms)}
          </Show>
        </span>
        <Show when={degraded()}>
          <span class="hb-chart-flag">
            {props.heartbeat.stale ? 'stale' : `${props.heartbeat.consecutive_failures} failing`}
          </span>
        </Show>
      </div>

      <Show
        when={stats().count > 0}
        fallback={<div class="hb-chart-empty">no probe cycles recorded yet</div>}
      >
        <svg
          class="hb-chart-plot"
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={`End-to-end sync latency over the last ${stats().count} probes`}
        >
          <defs>
            <linearGradient id={`hb-fill-${gradientId}`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" class="hb-fill-top" />
              <stop offset="100%" class="hb-fill-bottom" />
            </linearGradient>
          </defs>

          {/* Peak reference. Drawn under the data so the line stays legible. */}
          <line
            class="hb-chart-grid"
            x1="0"
            y1={yOf(stats().max)}
            x2={W}
            y2={yOf(stats().max)}
            vector-effect="non-scaling-stroke"
          />

          <For each={segments()}>
            {(run) => (
              <>
                <path class="hb-chart-area" d={areaPath(run)} fill={`url(#hb-fill-${gradientId})`} />
                <path
                  class="hb-chart-line"
                  d={linePath(run)}
                  vector-effect="non-scaling-stroke"
                />
              </>
            )}
          </For>

          {/* Failed cycles: a band, since there is no latency to plot. */}
          <For each={failureBands()}>
            {(band) => (
              <>
                <rect
                  class="hb-chart-fail-band"
                  x={band.from}
                  y="0"
                  width={Math.max(band.to - band.from, 1)}
                  height={H}
                >
                  <title>{`${band.count} failed ${
                    band.count === 1 ? 'probe' : 'probes'
                  } · from ${formatRelativeTime(band.ts)}`}</title>
                </rect>
                {/* Leading edge: where measurement stopped. */}
                <line
                  class="hb-chart-fail-edge"
                  x1={band.from}
                  y1="0"
                  x2={band.from}
                  y2={H}
                  vector-effect="non-scaling-stroke"
                />
              </>
            )}
          </For>

          {/* Latest reading. A vertical rule rather than a dot: the plot is
              stretched horizontally to fill the row, which turns a circle into
              an ellipse — verticals are the one shape that survives it. */}
          <Show when={lastOk()}>
            {(s) => (
              <line
                class="hb-chart-latest"
                x1={xOf(samples().indexOf(s()))}
                y1={yOf(s().ms as number)}
                x2={xOf(samples().indexOf(s()))}
                y2={H}
                vector-effect="non-scaling-stroke"
              />
            )}
          </Show>
        </svg>

        <div class="hb-chart-foot">
          <span>{windowLabel()} ago</span>
          <span class="hb-chart-scale">
            <Show when={stats().max > 0}>
              peak {formatMs(stats().max)} · avg {formatMs(stats().avg)}
            </Show>
            <Show when={stats().failures > 0}>
              {` · ${stats().failures} failed`}
            </Show>
          </span>
          <span>now</span>
        </div>
      </Show>
    </div>
  );
}
