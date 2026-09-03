import { For, Show } from 'solid-js';
import { A } from '@solidjs/router';
import { Empty, PageHead, Panel, Pill, StatusDot } from '../components/Chrome';
import { BootstrapProgress } from '../components/BootstrapProgress';
import { ActivityStrip, OpBadge } from '../components/Actions';
import { AllSspsActions, SspActions, SspLogsLink } from '../components/ClusterActions';
import { formatCount, formatUptime } from '../lib/format';
import { sspTone } from '../lib/status';
import type { Overview } from '../api/types';

/** Every SSP, with its env, its actions, and a link to its log stream. */
export function Ssps(props: { data: Overview | undefined; refresh: () => void }) {
  return (
    <>
      <PageHead
        crumb="Dashboard"
        title="Sync processors"
        actions={
          <Show when={props.data}>
            {(d) => <AllSspsActions count={d().ssps.length} onDone={props.refresh} />}
          </Show>
        }
      />

      <div class="page-body">
        <ActivityStrip operations={props.data?.operations} />

        <Show when={props.data} fallback={<Empty>Loading…</Empty>}>
          {(data) => (
            <Show
              when={data().ssps.length > 0}
              fallback={
                <Panel>
                  <Empty>
                    No SSPs are registered with this scheduler, so no client is
                    receiving live updates.
                  </Empty>
                </Panel>
              }
            >
              <div class="grid grid-2">
                <For each={data().ssps}>
                  {(ssp) => (
                    <div class="card">
                      <div class="card-head">
                        <div>
                          <h2 class="mono" style={{ 'font-size': '13px' }}>
                            {ssp.id}
                          </h2>
                          <div class="card-sub">
                            {ssp.ip ?? 'no address'} · v{ssp.version}
                          </div>
                        </div>
                        <div class="row">
                          <OpBadge operations={data().operations} target={ssp.id} />
                          <Pill tone={sspTone(ssp.status)}>
                            <StatusDot tone={sspTone(ssp.status)} />
                            {ssp.status}
                          </Pill>
                        </div>
                      </div>

                      <Show when={ssp.status !== 'ready'}>
                        <div style={{ 'margin-bottom': '14px' }}>
                          <BootstrapProgress
                            ssp={ssp}
                            timeoutSecs={data().bootstrap_timeout_secs}
                          />
                        </div>
                      </Show>

                      <dl class="kv">
                        <dt>Live views</dt>
                        <dd>{formatCount(ssp.views)}</dd>
                        <dt>Uptime</dt>
                        <dd>{formatUptime(ssp.uptime_seconds)}</dd>
                        <dt>Last heartbeat</dt>
                        <dd>{formatUptime(ssp.last_heartbeat_seconds_ago)} ago</dd>
                        <dt>In this phase</dt>
                        <dd>{formatUptime(ssp.state_seconds)}</dd>
                        <dt>Buffered events</dt>
                        <dd>{formatCount(ssp.buffered_events)}</dd>
                      </dl>

                      <Show when={ssp.env && Object.keys(ssp.env).length > 0}>
                        <details style={{ 'margin-top': '14px' }}>
                          <summary class="faint" style={{ cursor: 'pointer', 'font-size': '12px' }}>
                            Environment
                          </summary>
                          <dl class="kv" style={{ 'margin-top': '10px' }}>
                            <For each={Object.entries(ssp.env!)}>
                              {([k, v]) => (
                                <>
                                  <dt>{k}</dt>
                                  <dd>{v}</dd>
                                </>
                              )}
                            </For>
                          </dl>
                        </details>
                      </Show>

                      <div class="spread" style={{ 'margin-top': '16px' }}>
                        <A
                          href={`/logs?source=ssp:${encodeURIComponent(ssp.id)}`}
                          class="btn btn-sm"
                        >
                          Stream logs
                        </A>
                        <SspActions ssp={ssp} size="sm" onDone={props.refresh} />
                        <SspLogsLink ssp={ssp} />
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          )}
        </Show>
      </div>
    </>
  );
}
