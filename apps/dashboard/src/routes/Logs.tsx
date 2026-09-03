import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js';
import { useSearchParams } from '@solidjs/router';
import { openStream } from '../api/client';
import { Empty, PageHead, Pill, StatusDot } from '../components/Chrome';
import { formatClock } from '../lib/format';
import type { LogLine, Overview } from '../api/types';

/** Above this many lines the oldest are dropped, so a long tail cannot grow
 *  without bound in the tab's memory. */
const MAX_LINES = 5000;

type Entry =
  | { kind: 'line'; line: LogLine; seq: number }
  | { kind: 'dropped'; count: number; seq: number };

/**
 * Live log tail for the scheduler or any SSP.
 *
 * Backends are deliberately absent from the source list: the scheduler reaches
 * them with HTTP health checks and has no pipe to their output, so offering
 * them here would be offering something that cannot work.
 */
export function Logs(props: { overview: Overview | undefined }) {
  const [params, setParams] = useSearchParams<{ source?: string }>();
  const [entries, setEntries] = createSignal<Entry[]>([]);
  const [connected, setConnected] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [follow, setFollow] = createSignal(true);
  const [levelFilter, setLevelFilter] = createSignal('');
  const [text, setText] = createSignal('');

  let viewport: HTMLDivElement | undefined;
  let seq = 0;

  const source = () => params.source || 'scheduler';

  const sources = () => [
    { value: 'scheduler', label: 'Scheduler' },
    ...(props.overview?.ssps ?? []).map((s) => ({
      value: `ssp:${s.id}`,
      label: `SSP · ${s.id}`,
    })),
  ];

  // Re-open the stream whenever the source changes. `createEffect` tracking
  // `source()` plus `onCleanup` inside it is what makes switching sources tear
  // the old stream down rather than accumulating them.
  createEffect(() => {
    const src = source();
    setEntries([]);
    setConnected(false);
    setError(null);
    seq = 0;

    const close = openStream(
      `/logs?source=${encodeURIComponent(src)}&tail=true`,
      {
        onOpen: () => {
          setConnected(true);
          setError(null);
        },
        onEvent: (event, data) => {
          if (event === 'line') {
            try {
              const line = JSON.parse(data) as LogLine;
              push({ kind: 'line', line, seq: seq++ });
            } catch {
              /* skip a malformed line rather than dropping the stream */
            }
          } else if (event === 'dropped') {
            push({ kind: 'dropped', count: Number(data) || 0, seq: seq++ });
          }
        },
        onError: (err) => {
          setConnected(false);
          setError(err instanceof Error ? err.message : 'Stream disconnected');
        },
      },
    );
    onCleanup(close);
  });

  const push = (entry: Entry) => {
    setEntries((prev) => {
      const next = prev.length >= MAX_LINES ? prev.slice(prev.length - MAX_LINES + 1) : prev.slice();
      next.push(entry);
      return next;
    });
    if (follow()) {
      // After paint, so the height is the post-append height.
      queueMicrotask(() => {
        if (viewport) viewport.scrollTop = viewport.scrollHeight;
      });
    }
  };

  const visible = () => {
    const lvl = levelFilter();
    const q = text().trim().toLowerCase();
    return entries().filter((e) => {
      if (e.kind === 'dropped') return true;
      if (lvl && e.line.level !== lvl) return false;
      if (!q) return true;
      return (
        e.line.message.toLowerCase().includes(q) ||
        e.line.target.toLowerCase().includes(q) ||
        (e.line.fields ?? '').toLowerCase().includes(q)
      );
    });
  };

  // Following means "stick to the bottom". Scrolling up turns it off, which is
  // what a reader inspecting older lines expects.
  const onScroll = () => {
    if (!viewport) return;
    const atBottom =
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 40;
    if (atBottom !== follow()) setFollow(atBottom);
  };

  return (
    <>
      <PageHead
        crumb="Dashboard"
        title="Logs"
        actions={
          <Pill tone={connected() ? 'live' : 'idle'}>
            <StatusDot tone={connected() ? 'ok' : 'idle'} />
            {connected() ? 'streaming' : 'connecting…'}
          </Pill>
        }
      />

      <div class="page-body">
      <Show when={error()}>
        <div class="banner">
          <span class="dot bad" />
          {error()}
        </div>
      </Show>

      <div class="panel" style={{ 'margin-bottom': '12px', padding: '14px' }}>
        <div class="row" style={{ gap: '12px', 'flex-wrap': 'wrap', 'align-items': 'flex-end' }}>
          <div style={{ 'min-width': '220px' }}>
            <label for="src">Source</label>
            <select
              id="src"
              value={source()}
              onChange={(e) => setParams({ source: e.currentTarget.value })}
            >
              <For each={sources()}>
                {(s) => <option value={s.value}>{s.label}</option>}
              </For>
            </select>
          </div>
          <div style={{ 'min-width': '130px' }}>
            <label for="lvl">Level</label>
            <select
              id="lvl"
              value={levelFilter()}
              onChange={(e) => setLevelFilter(e.currentTarget.value)}
            >
              <option value="">All</option>
              <option value="ERROR">Error</option>
              <option value="WARN">Warn</option>
              <option value="INFO">Info</option>
              <option value="DEBUG">Debug</option>
              <option value="TRACE">Trace</option>
            </select>
          </div>
          <div style={{ flex: '1', 'min-width': '200px' }}>
            <label for="q">Search</label>
            <input
              id="q"
              placeholder="message, target or field…"
              value={text()}
              onInput={(e) => setText(e.currentTarget.value)}
            />
          </div>
          <div style={{ 'align-self': 'flex-end' }}>
            <button
              class="btn"
              onClick={() => {
                setFollow(true);
                if (viewport) viewport.scrollTop = viewport.scrollHeight;
              }}
              disabled={follow()}
            >
              {follow() ? 'Following' : 'Follow'}
            </button>
          </div>
        </div>
      </div>

      <div class="logs" ref={viewport} onScroll={onScroll}>
        <Show
          when={visible().length > 0}
          fallback={
            <Empty>
              {connected() ? 'No lines match.' : 'Waiting for the stream…'}
            </Empty>
          }
        >
          <For each={visible()}>
            {(entry) =>
              entry.kind === 'dropped' ? (
                <div class="log-dropped">
                  … {entry.count} lines dropped (this viewer fell behind)
                </div>
              ) : (
                <div class="log-line">
                  <span class="log-ts">{formatClock(entry.line.ts)}</span>
                  <span class={`log-level ${entry.line.level}`}>
                    {entry.line.level}
                  </span>
                  <span class="log-target">{entry.line.target}</span>
                  <span>
                    {entry.line.message}
                    <Show when={entry.line.fields}>
                      <span class="log-fields"> {entry.line.fields}</span>
                    </Show>
                  </span>
                </div>
              )
            }
          </For>
        </Show>
      </div>

      <div class="ghost" style={{ 'margin-top': '10px', 'font-size': '11px' }}>
        Scheduler and SSP logs only. Backends are reached over HTTP health
        checks, so the scheduler has no access to their output.
      </div>
      </div>
    </>
  );
}
