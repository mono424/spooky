import { createSignal, createEffect } from 'solid-js';
import { TableList } from './TableList';
import { TableView } from './TableView';
import { Toast } from '../ui/Toast';
import { getPref, setPref } from '../../utils/prefs';
import { useDevTools } from '../../context/DevToolsContext';

export function DatabaseTab() {
  const { state, fetchTables, isSp00kyAvailable } = useDevTools();
  const [filter, setFilter] = createSignal('');
  const [source, setSource] = createSignal<'local' | 'remote'>('local'); // Default to local
  const [error, setError] = createSignal<string | null>(null);
  // Internal `_00_*` sync tables are hidden by default; the toggle persists.
  const [showInternal, setShowInternalSig] = createSignal(
    getPref('database.showInternalTables', false)
  );
  const setShowInternal = (v: boolean) => {
    setShowInternalSig(v);
    setPref('database.showInternalTables', v);
  };

  // Enumerate the tables for whichever source is selected (one guarded
  // `INFO FOR DB` per source). Re-runs on source switch so Remote shows remote
  // tables (not the local-only `_00_*`) and vice-versa.
  //
  // Gate on `isSp00kyAvailable()`: on first open the panel mounts before the
  // page's Sp00ky connection is detected, so an early `INFO FOR DB` times out
  // silently and the list stays empty until a reload. Tracking availability
  // makes this effect re-run once detection completes, so the tables show up
  // without a manual reload.
  createEffect(() => {
    if (!isSp00kyAvailable()) return;
    void fetchTables?.(source());
  });

  // The list shown depends on the source: backend push covers local; remote is
  // enumerated on demand.
  const tables = () => (source() === 'local' ? state.database.tables : state.database.remoteTables ?? []);

  const handleError = (msg: string) => {
    setError(msg);
  };

  return (
    <div style={{ display: 'flex', 'flex-direction': 'column', height: '100%', width: '100%' }}>
      {/* oxlint-disable-next-line no-non-null-assertion */}
      {error() && <Toast message={error()!} type="error" onDismiss={() => setError(null)} />}
      <div
        class="table-controls"
        style={{
          height: '25px',
          padding: '0 8px',
          'border-bottom': '1px solid var(--sys-color-divider)',
          display: 'flex',
          'align-items': 'center',
          gap: '8px',
          'box-sizing': 'border-box',
          'flex-shrink': 0,
          background: 'var(--sys-color-surface)',
        }}
      >
        <input
          class="dt-filter-input"
          type="text"
          placeholder="Filter..."
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
          style={{ flex: '1' }}
        />
        <select
          value={source()}
          onChange={(e) => setSource(e.currentTarget.value as 'local' | 'remote')}
          style={{
            height: '18px',
            background: 'transparent',
            border: '1px solid var(--sys-color-outline-variant, #555)',
            color: 'var(--sys-color-on-surface, #fff)',
            'border-radius': '9px',
            'font-size': '11px',
            padding: '0 8px',
            outline: 'none',
            cursor: 'pointer',
          }}
        >
          <option value="local">Local</option>
          <option value="remote">Remote</option>
        </select>
      </div>
      <div class="database-container">
        <TableList
          tables={tables()}
          showInternal={showInternal()}
          onToggleInternal={setShowInternal}
        />
        <TableView
          filter={filter()}
          setFilter={setFilter}
          source={source()}
          onError={handleError}
        />
      </div>
    </div>
  );
}
