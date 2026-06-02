import { For, Show } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';

const NA = '—';
const UNAVAILABLE = 'unavailable';

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
  match: 'In sync',
  drift: 'Version drift',
  unknown: 'Unknown',
  na: 'Reported',
};

export function VersionsTab() {
  const { state, refreshVersions } = useDevTools();

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
        detail: 'JS client vs server',
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

  const driftCount = () => rows().filter((r) => rowStatus(r) === 'drift').length;

  return (
    <div class="mcp-container">
      <div class="mcp-header">
        <h2>Versions</h2>
        <div class="mcp-header-controls">
          <Show
            when={driftCount() > 0}
            fallback={
              <div class="mcp-status-badge connected">
                <span class="status-dot active" />
                In sync
              </div>
            }
          >
            <div class="mcp-status-badge disconnected">
              <span class="status-dot inactive" />
              {driftCount()} drift
            </div>
          </Show>
          <button class="btn" onClick={() => refreshVersions()}>
            Refresh
          </button>
        </div>
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
              <div class="versions-row">
                <div class="versions-component">
                  <span class="versions-name">{row.component}</span>
                  <span class="versions-detail">{row.detail}</span>
                </div>
                <div class="versions-value" classList={{ muted: !isKnown(row.frontend) }}>
                  {row.frontend}
                </div>
                <div class="versions-value" classList={{ muted: !isKnown(row.backend) }}>
                  {row.backend}
                </div>
                <div class="versions-status" classList={{ [status]: true }}>
                  <Show when={status === 'match' || status === 'drift' || status === 'unknown'}>
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

      <div class="mcp-section">
        <p>
          Frontend versions are baked into the bundle at build time. Backend versions are read over
          HTTP from the ssp <code>/version</code> and <code>/info</code> endpoints (the SurrealDB
          server version and scheduler URL come from ssp <code>/info</code>). Unreachable components
          show <code>unavailable</code>; a red dot marks a frontend/backend mismatch.
        </p>
      </div>
    </div>
  );
}
