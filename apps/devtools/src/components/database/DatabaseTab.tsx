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
    <div class="database-container">
      {/* oxlint-disable-next-line no-non-null-assertion */}
      {error() && <Toast message={error()!} type="error" onDismiss={() => setError(null)} />}
      <TableList
        tables={tables()}
        showInternal={showInternal()}
        onToggleInternal={setShowInternal}
        storage={source() === 'local' ? state.database.storage : undefined}
      />
      <TableView
        filter={filter()}
        setFilter={setFilter}
        source={source()}
        setSource={setSource}
        onError={handleError}
      />
    </div>
  );
}
