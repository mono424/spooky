import { For, Show } from 'solid-js';
import { A } from '@solidjs/router';
import {
  Cell,
  Empty,
  KeyValue,
  PageHead,
  Panel,
  Pill,
  Rail,
  StatusDot,
} from '../components/Chrome';
import { Sparkline } from '../components/Sparkline';
import { BootstrapProgress } from '../components/BootstrapProgress';
import { ActivityStrip, OpBadge } from '../components/Actions';
import {
  SchedulerActions,
  SspActions,
  SspLogsLink,
} from '../components/ClusterActions';
import { formatClock, formatCount, formatMs, formatUptime, splitValue } from '../lib/format';
import { backendTone, schedulerTone, sspTone } from '../lib/status';
import type { Overview as OverviewData, PresenceSample } from '../api/types';

/**
 * One count over time, in the same chrome as the latency chart beside it.
 *
 * The current value rides in the panel's action slot rather than as a big
 * number above the plot: four panels in a row have to agree on their internal
 * layout or the sparklines stop sharing a baseline, which is the whole point of
 * putting them on one band.
 */
function PresenceChart(props: {
  label: string;
  sub: string;
  ready: boolean;
  value: number | undefined;
  points: { ts: number; ms: number; ok: true }[];
}) {
  return (
    <Panel
      title={props.label}
      sub={props.sub}
      actions={
        <Show when={props.ready}>
          <Pill>{formatCount(props.value)} now</Pill>
        </Show>
      }
    >
      <Show
        when={props.ready}
        fallback={<Empty>Waiting for the first presence sample…</Empty>}
      >
        <Sparkline
          points={props.points}
          height={72}
          format={formatCount}
          ariaLabel={props.label}
        />
      </Show>
    </Panel>
  );
}

/**
 * Is the cluster healthy, and how fast is a write reaching a client right now.
 *
 * The latency reading is the scheduler's own heartbeat probe: it writes
 * `_00_heartbeat:probe` upstream and times the full round trip through
 * `/ingest`, the WAL, the broadcast, and every SSP's circuit. It measures the
 * real sync path rather than a synthetic ping, which is why it earns the
 * largest readout on the page.
 */
export function Overview(props: {
  data: OverviewData | undefined;
  error?: string;
  /** Re-poll the overview now, e.g. right after an action. */
  refresh: () => void;
}) {
  const sched = () => props.data?.scheduler;
  const heartbeat = () => sched()?.heartbeat;

  const points = () =>
    (heartbeat()?.samples ?? []).map((s) => ({ ts: s.ts, ms: s.ms, ok: s.ok }));

  // The presence sampler's ring, folded into this same poll — so every readout
  // and chart below costs no request of its own and no database work.
  const presence = () => props.data?.presence;
  const totals = () => presence()?.totals;
  const series = (pick: (s: PresenceSample) => number) =>
    (presence()?.samples ?? []).map((s) => ({
      ts: s.ts,
      ms: pick(s),
      ok: true as const,
    }));

  const failed = () => points().filter((p) => !p.ok);
  const lastFailure = () => {
    const f = failed();
    return f.length ? f[f.length - 1]!.ts : null;
  };

  const lag = () => sched()?.lag ?? 0;
  const latency = () => splitValue(formatMs(heartbeat()?.last_e2e_ms));

  return (
    <>
      <PageHead
        crumb="Dashboard"
        title="Overview"
        actions={
          <Show when={sched()}>
            {(s) => (
              <Pill tone={schedulerTone(s().status)} dot>
                scheduler {s().status}
              </Pill>
            )}
          </Show>
        }
      />

      <div class="page-body">
        <Show when={props.error}>
          <div class="banner">
            <span class="dot bad" />
            {props.error}
          </div>
        </Show>

        <ActivityStrip operations={props.data?.operations} />

        <Show when={props.data} fallback={<Empty>Loading…</Empty>}>
          {(data) => (
            <div class="stack">
              <Rail>
                <Cell
                  label="E2E latency"
                  tone={heartbeat()?.stale ? 'warn' : 'ok'}
                  value={latency().value}
                  unit={latency().unit}
                  foot={
                    <Show
                      when={heartbeat()?.enabled}
                      fallback="probe disabled"
                    >
                      {heartbeat()?.blocked_reason ??
                        `${heartbeat()?.consecutive_failures ?? 0} consecutive failures`}
                    </Show>
                  }
                />
                <Cell
                  label="SSPs ready"
                  tone={
                    data().totals.ssps === 0
                      ? 'bad'
                      : data().totals.ssps_ready === data().totals.ssps
                        ? 'ok'
                        : 'warn'
                  }
                  value={`${data().totals.ssps_ready}/${data().totals.ssps}`}
                  foot={`${formatCount(sched()?.views ?? 0)} live views`}
                />
                <Cell
                  label="Backends"
                  tone={
                    data().totals.backends === 0
                      ? 'idle'
                      : data().totals.backends_healthy === data().totals.backends
                        ? 'ok'
                        : 'bad'
                  }
                  value={`${data().totals.backends_healthy}/${data().totals.backends}`}
                  foot={
                    data().totals.backends === 0
                      ? 'none configured'
                      : 'healthy'
                  }
                />
                <Cell
                  label="Ingest lag"
                  tone={lag() > 1000 ? 'warn' : 'ok'}
                  value={formatCount(lag())}
                  unit="events"
                  foot={`seq ${formatCount(sched()?.latest_seq ?? 0)}`}
                />
              </Rail>

              {/* Who is actually using the stack, on its own band. Every figure
                  here counts only rows still inside `lastActiveAt + ttl`. */}
              <Show when={presence()?.ready}>
                <Rail>
                  <Cell
                    label="Users"
                    value={formatCount(totals()?.users)}
                    foot="signed in"
                  />
                  <Cell
                    label="Sessions"
                    value={formatCount(totals()?.sessions)}
                    foot={`${formatCount(totals()?.anon_sessions)} anonymous`}
                  />
                  <Cell label="Views" value={formatCount(totals()?.views)} />
                  <Cell
                    label="Shared"
                    value={formatCount(totals()?.shared_views)}
                    foot="more than one subscriber"
                  />
                  <Cell
                    label="Slow"
                    value={formatCount(totals()?.slow_views)}
                    tone={totals()?.slow_views ? 'warn' : undefined}
                    foot="materialization p99"
                  />
                  <Cell
                    label="Errored"
                    value={formatCount(totals()?.errored_views)}
                    tone={totals()?.errored_views ? 'bad' : undefined}
                  />
                </Rail>
              </Show>

              {/* One band, four series. Each keeps its own scale: views
                  outnumber users by one to two orders of magnitude, and a
                  shared axis would flatten the users line onto the baseline. */}
              <div class="grid grid-4">
                <Panel
                  title="Sync round trip"
                  sub="DB write → SSP circuit, per probe"
                  actions={
                    <Show when={failed().length > 0}>
                      <span
                        class="pill bad"
                        title={
                          lastFailure()
                            ? `Last failed cycle at ${formatClock(lastFailure()!)}`
                            : undefined
                        }
                      >
                        {failed().length} failed of {points().length}
                      </span>
                    </Show>
                  }
                >
                  <Sparkline points={points()} height={72} />
                </Panel>

                <PresenceChart
                  label="Users"
                  sub="signed in, fading out over ~9m"
                  ready={!!presence()?.ready}
                  value={totals()?.users}
                  points={series((s) => s.users)}
                />
                <PresenceChart
                  label="Sessions"
                  sub="one per open tab"
                  ready={!!presence()?.ready}
                  value={totals()?.sessions}
                  points={series((s) => s.sessions)}
                />
                <PresenceChart
                  label="Registered views"
                  sub="one per session, per query"
                  ready={!!presence()?.ready}
                  value={totals()?.views}
                  points={series((s) => s.views)}
                />
              </div>

              <div class="grid grid-2">
                <Panel
                  title="Scheduler"
                  sub={sched()?.id}
                  actions={<SchedulerActions size="sm" onDone={props.refresh} />}
                >
                  <KeyValue
                    rows={[
                      [
                        'Status',
                        <Pill tone={schedulerTone(sched()?.status)}>
                          {sched()?.status ?? '—'}
                        </Pill>,
                      ],
                      ['Version', sched()?.version ?? '—'],
                      ['SurrealDB', sched()?.surrealdb_version ?? '—'],
                      ['Address', sched()?.ip ?? '—'],
                      ['Uptime', formatUptime(sched()?.uptime_seconds)],
                      ['Snapshot seq', formatCount(sched()?.snapshot_seq)],
                      ['Pending events', formatCount(sched()?.pending_events)],
                    ]}
                  />
                </Panel>

                <Panel
                  title="Sync processors"
                sub="Queries are pinned per SSP, so one lagging processor is a real outage for its clients"
                actions={
                  <A href="/ssps" class="btn btn-sm">
                    all
                  </A>
                }
                flush
              >
                <Show
                  when={data().ssps.length > 0}
                  fallback={
                    <Empty>
                      No SSPs registered. Nothing is serving live queries.
                    </Empty>
                  }
                >
                  <div class="table-scroll">
                    <table>
                      <thead>
                        <tr>
                          <th>SSP</th>
                          <th>Status</th>
                          <th>Views</th>
                          <th>Uptime</th>
                          <th style={{ width: '30%' }}>Progress</th>
                          <th />
                        </tr>
                      </thead>
                      <tbody>
                        <For each={data().ssps}>
                          {(ssp) => (
                            <tr>
                              <td>
                                <div class="row">
                                  <StatusDot
                                    tone={sspTone(ssp.status)}
                                    pulse={ssp.status !== 'ready'}
                                  />
                                  <span>{ssp.id}</span>
                                </div>
                                <div class="id" style={{ 'margin-top': '2px' }}>
                                  {ssp.ip ?? '—'} · v{ssp.version}
                                </div>
                              </td>
                              <td data-label="Status">
                                <Pill tone={sspTone(ssp.status)}>
                                  {ssp.status}
                                </Pill>
                              </td>
                              <td data-label="Views">{formatCount(ssp.views)}</td>
                              <td class="dim" data-label="Uptime">
                                {formatUptime(ssp.uptime_seconds)}
                              </td>
                              <td data-label="Progress">
                                <Show
                                  when={ssp.status !== 'ready'}
                                  fallback={
                                    <div class="row">
                                      <span class="ghost">caught up</span>
                                      <OpBadge
                                        operations={data().operations}
                                        target={ssp.id}
                                      />
                                    </div>
                                  }
                                >
                                  <BootstrapProgress
                                    ssp={ssp}
                                    timeoutSecs={data().bootstrap_timeout_secs}
                                  />
                                </Show>
                              </td>
                              <td data-label="Actions" style={{ 'text-align': 'right' }}>
                                <SspActions ssp={ssp} size="sm" onDone={props.refresh} />
                                <SspLogsLink ssp={ssp} />
                              </td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
                </Panel>
              </div>

              <Show when={data().backends.length > 0}>
                <Panel
                  title="Backends"
                  sub="Health-checked application services"
                  actions={
                    <A href="/backends" class="btn btn-sm">
                      all
                    </A>
                  }
                  flush
                >
                  <div class="table-scroll">
                    <table>
                      <thead>
                        <tr>
                          <th>Name</th>
                          <th>Status</th>
                          <th>Response</th>
                          <th>URL</th>
                        </tr>
                      </thead>
                      <tbody>
                        <For each={data().backends}>
                          {(b) => (
                            <tr>
                              <td>
                                <A href={`/backends/${encodeURIComponent(b.name)}`}>
                                  <div class="row">
                                    <StatusDot tone={backendTone(b.status)} />
                                    {b.name}
                                  </div>
                                </A>
                              </td>
                              <td data-label="Status">
                                <Pill tone={backendTone(b.status)}>
                                  {b.status}
                                </Pill>
                              </td>
                              <td class="dim" data-label="Response">{formatMs(b.response_time_ms)}</td>
                              <td class="ghost truncate" data-label="URL">{b.url}</td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Panel>
              </Show>
            </div>
          )}
        </Show>
      </div>
    </>
  );
}
