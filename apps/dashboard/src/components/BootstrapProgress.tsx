import { Show } from 'solid-js';
import { formatCount, formatUptime } from '../lib/format';
import type { SspEntity } from '../api/types';

/**
 * How far along a bootstrapping or replaying SSP is.
 *
 * Two fidelities, because progress reporting is advisory and an older SSP
 * simply does not send it:
 *
 *  - With `bootstrap`, a real fraction of tables loaded.
 *  - Without it, the phase and how long it has been in it, against the
 *    scheduler's bootstrap budget. The bar is drawn indeterminate rather than
 *    inventing a percentage from elapsed time — that would read as progress
 *    when it is only patience.
 */
export function BootstrapProgress(props: {
  ssp: SspEntity;
  timeoutSecs: number;
}) {
  const boot = () => props.ssp.bootstrap;

  const fraction = () => {
    const b = boot();
    if (!b || b.tables_total === 0) return null;
    return Math.min(1, b.tables_done / b.tables_total);
  };

  // The budget the scheduler reaps a hung bootstrap on. Worth showing, because
  // an SSP close to it is about to be evicted and restarted.
  const overdue = () =>
    props.ssp.state_seconds !== null &&
    props.ssp.state_seconds > props.timeoutSecs;

  return (
    <div>
      <div class="bar">
        <div
          class="bar-fill"
          classList={{ indeterminate: fraction() === null }}
          style={{ width: fraction() !== null ? `${fraction()! * 100}%` : undefined }}
        />
      </div>

      <div
        class="spread faint"
        style={{ 'margin-top': '6px', 'font-size': '11.5px' }}
      >
        <span>
          <Show
            when={boot()}
            fallback={
              props.ssp.status === 'replaying'
                ? `Replaying ${formatCount(props.ssp.buffered_events)} buffered events`
                : 'Loading circuit…'
            }
          >
            {(b) => (
              <>
                {b().tables_done}/{b().tables_total} tables
                <Show when={b().current_table}>
                  {' · '}
                  <span class="mono">{b().current_table}</span>
                </Show>
                {' · '}
                {formatCount(b().rows_loaded)} rows
              </>
            )}
          </Show>
        </span>
        <span classList={{ dim: !overdue() }} style={overdue() ? { color: 'var(--bad)' } : undefined}>
          {formatUptime(props.ssp.state_seconds)}
          {' / '}
          {formatUptime(props.timeoutSecs)}
        </span>
      </div>
    </div>
  );
}
