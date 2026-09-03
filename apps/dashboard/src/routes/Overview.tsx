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
import { formatCount, formatMs, formatUptime, splitValue } from '../lib/format';
import { backendTone, schedulerTone, sspTone } from '../lib/status';
import type { Overview as OverviewData } from '../api/types';

/**
 * Is the cluster healthy, and how fast is a write reaching a client right now.
 *
 * The latency reading is the scheduler's own heartbeat probe: it writes
 * `_00_heartbeat:probe` upstream and times the full round trip through
 * `/ingest`, the WAL, the broadcast, and every SSP's circuit. It measures the
 * real sync path rather than a synthetic ping, which is why it earns the
 * largest readout on the page.
 */
export function Overview(props: { data: OverviewData | undefined; error?: string }) {
  const sched = () => props.data?.scheduler;
  const heartbeat = () => sched()?.heartbeat;

  const points = () =>
    (heartbeat()?.samples ?? []).map((s) => ({ ts: s.ts, ms: s.ms, ok: s.ok }));

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

              <div class="grid grid-2">
                <Panel
                  title="Sync round trip"
                  sub="DB write → scheduler → SSP circuit, per probe cycle"
                  actions={
                    <Show when={points().some((p) => !p.ok)}>
                      <span class="pill bad">
                        {points().filter((p) => !p.ok).length} failed
                      </span>
                    </Show>
                  }
                >
                  <Sparkline points={points()} height={78} />
                </Panel>

                <Panel title="Scheduler" sub={sched()?.id}>
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
              </div>

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
                          <th style={{ width: '36%' }}>Progress</th>
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
                                    <span class="ghost">caught up</span>
                                  }
                                >
                                  <BootstrapProgress
                                    ssp={ssp}
                                    timeoutSecs={data().bootstrap_timeout_secs}
                                  />
                                </Show>
                              </td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
              </Panel>

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
