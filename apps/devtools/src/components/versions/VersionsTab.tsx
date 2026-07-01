import { For, Show } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatDuration } from '../../utils/formatters';
import type { BackendEntity } from '../../types/devtools';

const NA = '—';
const UNAVAILABLE = 'unavailable';

/** Compact uptime label from a seconds count (e.g. 90 -> "1.5m"). */
function formatUptime(seconds: number | undefined): string | null {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds)) return null;
  return formatDuration(seconds * 1000);
}

/** Map a backend status string to the shared status-dot modifier class. */
function statusDotClass(status: string | undefined): 'active' | 'inactive' | '' {
  switch (status) {
    case 'ready':
    case 'healthy':
      return 'active';
    case 'failed':
    case 'unhealthy':
      return 'inactive';
    default:
      return '';
  }
}

type RowStatus = 'match' | 'drift' | 'unknown' | 'na';

interface VersionRow {
  component: string;
  detail: string;
  frontend: string;
  backend: string;
  /** Whether frontend/backend are the same artifact and should be compared. */
  compare: boolean;
}

function isKnown(v: string): boolean {
  return v !== NA && v !== UNAVAILABLE && v.length > 0;
}

function rowStatus(row: VersionRow): RowStatus {
  if (row.compare) {
    if (!isKnown(row.frontend) || !isKnown(row.backend)) return 'unknown';
    return row.frontend === row.backend ? 'match' : 'drift';
  }
  // Single-sided rows: reflect whether the meaningful side is known.
  const meaningful = isKnown(row.frontend) ? row.frontend : row.backend;
  return isKnown(meaningful) ? 'na' : 'unknown';
}

const STATUS_LABEL: Record<RowStatus, string> = {
  match: 'in sync',
  drift: 'drift',
  unknown: '·',
  na: '·',
};

/** Ordered key facts to render per stack entity (skipped when absent). */
function entityFacts(e: BackendEntity): { label: string; value: string }[] {
  const facts: { label: string; value: string }[] = [];
  const uptime = formatUptime(e.uptime_seconds);
  if (e.surrealdb_version) facts.push({ label: 'surrealdb', value: String(e.surrealdb_version) });
  if (typeof e.views === 'number') facts.push({ label: 'views', value: String(e.views) });
  if (uptime) facts.push({ label: 'uptime', value: uptime });
  if (e.ip) facts.push({ label: 'ip', value: String(e.ip) });
  return facts;
}

export function VersionsTab() {
  const { state } = useDevTools();

  const entities = (): BackendEntity[] => state.versions.entities ?? [];

  const rows = (): VersionRow[] => {
    const v = state.versions;
    return [
      {
        component: 'wasm core',
        detail: 'bundled ssp-wasm vs running ssp circuit',
        frontend: v.frontend.wasm,
        backend: v.backend.ssp,
        compare: true,
      },
      {
        component: 'surrealdb',
        detail: 'in-browser WASM engine vs server engine',
        frontend: v.frontend.surrealdb,
        backend: v.backend.surrealdb,
        compare: true,
      },
      {
        component: 'ssp',
        detail: 'sync provider service',
        frontend: NA,
        backend: v.backend.ssp,
        compare: false,
      },
      {
        component: 'scheduler',
        detail: 'job scheduler service',
        frontend: NA,
        backend: v.backend.scheduler,
        compare: false,
      },
      {
        component: 'sp00ky core',
        detail: '@spooky-sync/core bundle',
        frontend: v.frontend.core,
        backend: NA,
        compare: false,
      },
    ];
  };

  return (
    <div class="mcp-container">
      <div class="mcp-header">
        <h2>Versions</h2>
      </div>

      <div class="versions-table">
        <div class="versions-row versions-head">
          <div>Component</div>
          <div>Frontend</div>
          <div>Backend</div>
          <div>Status</div>
        </div>
        <For each={rows()}>
          {(row) => {
            const status = rowStatus(row);
            return (
              <div class="versions-row" classList={{ drift: status === 'drift' }}>
                <div class="versions-component" title={row.detail}>
                  <span class="versions-name">{row.component}</span>
                </div>
                <div
                  class="versions-value"
                  classList={{ muted: !isKnown(row.frontend) }}
                  title={row.frontend}
                >
                  <bdi>{row.frontend}</bdi>
                </div>
                <div
                  class="versions-value"
                  classList={{ muted: !isKnown(row.backend) }}
                  title={row.backend}
                >
                  <bdi>{row.backend}</bdi>
                </div>
                <div class="versions-status" classList={{ [status]: true }}>
                  <Show when={status === 'match' || status === 'drift'}>
                    <span
                      class="status-dot"
                      classList={{ active: status === 'match', inactive: status === 'drift' }}
                    />
                  </Show>
                  <span>{STATUS_LABEL[status]}</span>
                </div>
              </div>
            );
          }}
        </For>
      </div>

      <Show when={entities().length > 0}>
        <div class="versions-stack">
          <div class="versions-stack-head">Stack</div>
          <For each={entities()}>
            {(e) => (
              <div class="versions-stack-row">
                <div class="versions-stack-id">
                  <span
                    class="status-dot"
                    classList={{
                      active: statusDotClass(e.status) === 'active',
                      inactive: statusDotClass(e.status) === 'inactive',
                    }}
                  />
                  <span class="versions-stack-entity">{e.entity}</span>
                  <Show when={e.id}>
                    <span class="versions-stack-detail" title={e.id}>
                      {e.id}
                    </span>
                  </Show>
                </div>
                <div class="versions-stack-meta">
                  <Show when={e.status}>
                    <span class="versions-stack-status">{e.status}</span>
                  </Show>
                  <span class="versions-stack-version" classList={{ muted: !e.version }}>
                    <bdi>{e.version ?? NA}</bdi>
                  </span>
                </div>
                <div class="versions-stack-facts">
                  <For each={entityFacts(e)}>
                    {(f) => (
                      <span class="versions-fact">
                        <span class="versions-fact-label">{f.label}</span>
                        <span class="versions-fact-value">{f.value}</span>
                      </span>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <div class="mcp-section">
        <p>
          Frontend versions are baked into the bundle at build time. Backend versions and the Stack
          info come from the <code>fn::spooky::info()</code> function (the same data the{' '}
          <code>/info</code> endpoint exposes), called over the live connection. Unreachable
          components show <code>unavailable</code>; a red dot marks a frontend/backend mismatch.
        </p>
      </div>
    </div>
  );
}
