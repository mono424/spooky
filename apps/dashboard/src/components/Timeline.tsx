import { For, Show, createMemo, createSignal } from 'solid-js';
import type { JSX } from 'solid-js';
import { formatDuration } from '../lib/format';

/**
 * A Gantt of a run's steps on a shared time axis.
 *
 * This is the view that answers the question an operator actually opens a run
 * to ask: *where did the time go, and what is it stuck behind?* A status column
 * cannot answer that; bar position and length can, at a glance.
 *
 * Design decisions worth not undoing:
 *
 *  - **One axis for the whole run.** Every lane is scaled to the same window,
 *    so a step that took 40x longer than its neighbour looks 40x longer. Lanes
 *    scaled individually would be prettier and would lie.
 *  - **A step that never ran is hatched, not empty.** Zero width would read as
 *    "instantaneous" rather than "never happened".
 *  - **An unfinished step fades out to the right** instead of ending at `now`,
 *    because it has no end yet and a hard edge would claim one.
 *  - **Sub-millisecond steps still get a visible stub.** A real event that is
 *    too fast to plot must not become invisible.
 *  - **An inferred start has an open left edge.** A step whose start was never
 *    recorded is placed at the earliest instant it could have run; the fade
 *    says so, rather than drawing a hard edge on a guess.
 */

export interface Lane {
  id: string;
  label: string;
  /** Epoch ms. `null` when the step never started. */
  start: number | null;
  /**
   * `start` was inferred rather than recorded — see `WorkflowDetail.lanes`. The
   * bar is drawn with an open left edge so it does not claim a precision it
   * does not have, for the same reason a never-run step is hatched.
   */
  estimated?: boolean;
  /** Epoch ms. `null` while still running, or if it never started. */
  end: number | null;
  status: string;
  tone: 'ok' | 'warn' | 'bad' | 'idle';
  /** Names this lane waited on, shown in the gutter. */
  dependsOn?: string[];
  /** Rendered when the lane is expanded. */
  detail?: () => JSX.Element;
}

const TICKS = 5;

export function Timeline(props: {
  lanes: Lane[];
  /** Run window. `windowEnd` null = still running, so the axis ends at now. */
  windowStart: number | null;
  windowEnd: number | null;
}) {
  const [open, setOpen] = createSignal<string | null>(null);

  const span = createMemo(() => {
    const lanes = props.lanes;
    // Derive the window from the lanes as well as the run, so a step that
    // somehow falls outside the run's own stamps is still visible rather than
    // clipped off the edge.
    const starts = lanes
      .map((l) => l.start)
      .filter((v): v is number => typeof v === 'number');
    const ends = lanes
      .map((l) => l.end)
      .filter((v): v is number => typeof v === 'number');

    const candidatesStart = [props.windowStart, ...starts].filter(
      (v): v is number => typeof v === 'number',
    );
    const candidatesEnd = [props.windowEnd, ...ends].filter(
      (v): v is number => typeof v === 'number',
    );

    const from = candidatesStart.length ? Math.min(...candidatesStart) : 0;
    // An unfinished run's axis runs to now, so an in-flight bar keeps growing.
    const rawTo = candidatesEnd.length ? Math.max(...candidatesEnd) : from;
    const to = props.windowEnd === null ? Math.max(rawTo, Date.now()) : rawTo;

    // Never divide by zero: a run that started and ended inside the same
    // millisecond still needs an axis.
    return { from, to, total: Math.max(1, to - from) };
  });

  const pct = (t: number) => {
    const { from, total } = span();
    return Math.min(100, Math.max(0, ((t - from) / total) * 100));
  };

  const ticks = createMemo(() => {
    const { total } = span();
    return Array.from({ length: TICKS }, (_, i) => {
      const at = (i / (TICKS - 1)) * 100;
      return { at, label: formatDuration((total * i) / (TICKS - 1)) };
    });
  });

  /** The tick rules, continued down through every lane as a background. */
  const gridImage = createMemo(() => {
    const stops = ticks()
      .slice(1, -1)
      .map(
        (t) =>
          `linear-gradient(to right, transparent calc(${t.at}% - 1px), var(--rule) ${t.at}%, transparent calc(${t.at}% + 1px))`,
      );
    return stops.join(', ') || 'none';
  });

  const geometry = (lane: Lane) => {
    if (lane.start === null) return null;
    const left = pct(lane.start);
    const running = lane.end === null;
    const right = pct(running ? span().to : lane.end!);
    return {
      left,
      // A hairline-thin bar is unreadable, so give every real span a floor.
      width: Math.max(0.6, right - left),
      running,
      ms: running ? span().to - lane.start : lane.end! - lane.start,
    };
  };

  return (
    <div class="tl">
      <div class="tl-axis">
        <For each={ticks()}>
          {(t, i) => (
            <div class="tl-tick" style={{ left: `${t.at}%` }}>
              <Show when={i() < TICKS - 1}>
                <span>+{t.label}</span>
              </Show>
            </div>
          )}
        </For>
      </div>

      <Show
        when={props.lanes.length > 0}
        fallback={<div class="tl-empty">No steps recorded for this run.</div>}
      >
        <For each={props.lanes}>
          {(lane) => {
            const g = () => geometry(lane);
            const isOpen = () => open() === lane.id;
            return (
              <div class="tl-row" classList={{ open: isOpen() }}>
                <div
                  class="tl-label"
                  onClick={() => setOpen(isOpen() ? null : lane.id)}
                  title={lane.label}
                >
                  <span class="dot" classList={{ [lane.tone]: true }} />
                  <span class="tl-name">{lane.label}</span>
                  <Show when={lane.dependsOn?.length}>
                    <span class="tl-dep" title={`waits on ${lane.dependsOn!.join(', ')}`}>
                      ←{lane.dependsOn!.length}
                    </span>
                  </Show>
                </div>

                <div
                  class="tl-track"
                  style={{ '--tl-grid': gridImage() }}
                  onClick={() => setOpen(isOpen() ? null : lane.id)}
                >
                  <Show
                    when={g()}
                    fallback={
                      /* Never started: a hatched stub at the origin, plus the
                         reason, rather than an empty lane. */
                      <>
                        <div
                          class="tl-bar idle"
                          style={{ left: '0%', width: '2.5%' }}
                        />
                        <span class="tl-duration" style={{ left: 'calc(2.5% + 8px)' }}>
                          {lane.status}
                        </span>
                      </>
                    }
                  >
                    {(geo) => (
                      <>
                        <div
                          class="tl-bar"
                          classList={{
                            [lane.tone]: true,
                            running: geo().running,
                            estimated: !!lane.estimated,
                          }}
                          style={{
                            left: `${geo().left}%`,
                            width: `${geo().width}%`,
                          }}
                          title={
                            lane.estimated
                              ? 'Start time was not recorded for this step; drawn from when its dependencies finished.'
                              : undefined
                          }
                        />
                        {/* Duration label sits after the bar, or before it once
                            the bar runs close to the right edge. */}
                        <span
                          class="tl-duration"
                          style={
                            geo().left + geo().width > 82
                              ? { right: `calc(${100 - geo().left}% + 8px)` }
                              : { left: `calc(${geo().left + geo().width}% + 8px)` }
                          }
                        >
                          {formatDuration(geo().ms)}
                          {geo().running ? '…' : ''}
                        </span>
                      </>
                    )}
                  </Show>
                </div>

                <Show when={isOpen() && lane.detail}>
                  <div class="tl-detail">{lane.detail!()}</div>
                </Show>
              </div>
            );
          }}
        </For>
      </Show>
    </div>
  );
}
