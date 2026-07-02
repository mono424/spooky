import { For, Show, createEffect, createMemo, createSignal } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { escapeHtml } from '../../utils/html';
import { Cell } from './Cell';
import { JsonView } from '../ui/JsonView';

function getRecordId(row: Record<string, unknown>): string | null {
  if (!row.id) return null;
  if (typeof row.id === 'string') return row.id;
  if (typeof row.id === 'object' && row.id !== null) return row.id.toString();
  return String(row.id);
}

function CopyIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="9" y="9" width="12" height="12" rx="2" ry="2"></rect>
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="20 6 9 17 4 12"></polyline>
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
      <line x1="6" y1="6" x2="18" y2="18"></line>
      <line x1="18" y1="6" x2="6" y2="18"></line>
    </svg>
  );
}

/** Chrome-style circular refresh arrow. */
function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
      <path d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z" />
    </svg>
  );
}

interface TableViewProps {
  filter: string;
  setFilter: (val: string) => void;
  source: 'local' | 'remote';
  setSource: (val: 'local' | 'remote') => void;
  onError?: (msg: string) => void;
}

export function TableView(props: TableViewProps) {
  const { selectedTable, setSelectedTable, runQuery } = useDevTools();
  // Filter and source are now props

  // Row inspected in the bottom JSON pane
  const [inspectedRow, setInspectedRow] = createSignal<Record<string, unknown> | null>(null);
  const [copied, setCopied] = createSignal(false);
  // Bumped by the toolbar refresh button to re-run the fetch effect.
  const [refreshTick, setRefreshTick] = createSignal(0);

  const copyInspected = async () => {
    const row = inspectedRow();
    if (!row) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(row, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error('[TableView] Copy failed:', err);
    }
  };

  // Fetch table data when a table is selected, source changes, or refresh clicked
  createEffect(() => {
    const table = selectedTable();
    const currentSource = props.source;
    refreshTick();
    setInspectedRow(null); // close the JSON pane when the table/source changes
    if (table && runQuery) {
      // Construct query: SELECT * FROM table LIMIT 20
      setLoading(true);
      console.log('[TableView] Triggering query for table:', table, 'Source:', currentSource);
      runQuery(`SELECT * FROM ${table} LIMIT 20`, currentSource)
        .then((result: any) => {
          console.log('[TableView] Query result:', result);
          if (Array.isArray(result)) {
            // Unwrap SurrealDB result format [{ status: 'OK', result: [...] }]
            // SurrealDB can return different formats depending on version and transport.
            // Case 1: Wrapped in result object, single statement
            if (
              result.length > 0 &&
              result[0] &&
              typeof result[0] === 'object' &&
              'result' in result[0]
            ) {
              const queryResult = result[0].result;
              setData(Array.isArray(queryResult) ? queryResult : []);
            }
            // Case 2: Legacy/Flattened [[...]] (Double array)
            else if (result.length > 0 && Array.isArray(result[0])) {
              setData(result[0]);
            }
            // Case 3: Array of records directly (already unwrapped or different driver)
            else if (result.length > 0) {
              setData(result);
            }
            // Case 4: Empty array -> Empty table
            else if (result.length === 0) {
              setData([]);
            } else {
              // Fallback
              console.warn('[TableView] Unexpected result format', result);
              setData([]);
            }
          } else {
            console.warn('[TableView] Result is not an array', result);
            // If result is { result: ... } (single object, not array of results)
            if (result && typeof result === 'object' && 'result' in result) {
              const queryResult = result.result;
              setData(Array.isArray(queryResult) ? queryResult : []);
            } else {
              setData([]);
            }
          }
          return undefined;
        })
        .catch((err) => {
          console.error('[TableView] Query Error:', err);
          let msg =
            err instanceof Error
              ? err.message
              : typeof err === 'string'
                ? err
                : JSON.stringify(err);
          if (!msg) {
            msg = `EMPTY ERROR OBJ: ${String(err)} type=${typeof err}`;
          }
          props.onError?.(msg);
          setData([]);
        })
        .finally(() => setLoading(false));
    }
  });

  const [data, setData] = createSignal<Record<string, unknown>[]>([]);
  const [loading, setLoading] = createSignal(false);

  const tableData = createMemo(() => {
    let currentData = data();
    const filterText = props.filter.toLowerCase();
    if (filterText) {
      currentData = currentData.filter((row) => {
        return Object.values(row).some((val) => String(val).toLowerCase().includes(filterText));
      });
    }
    return currentData;
  });

  const columns = createMemo(() => {
    const table = selectedTable();
    const schemaCols = (table && useDevTools().state.database.schema?.[table]) || [];

    const fullData = data();
    const dataKeys = new Set<string>();

    if (fullData && fullData.length > 0) {
      fullData.forEach((row) => {
        if (row && typeof row === 'object') {
          Object.keys(row).forEach((key) => dataKeys.add(key));
        }
      });
    }

    // Merge schema columns and data columns
    const allKeys = new Set([...schemaCols, ...dataKeys]);

    const finalCols = Array.from(allKeys).toSorted((a, b) => {
      // ID always first
      if (a.toLowerCase() === 'id') return -1;
      if (b.toLowerCase() === 'id') return 1;

      // Schema columns next (preserve order if possible, but sets are unordered)
      // We can prioritize schema columns if we want, but alpha sort is standard
      return a.localeCompare(b);
    });

    console.log('[TableView] Columns:', finalCols);
    console.log('[TableView] Data Sample (first row):', fullData?.[0]);

    return finalCols;
  });

  return (
    <div class="database-data">
      <div class="table-data">
        <Show
          when={selectedTable()}
          fallback={<div class="empty-state">Select a table to view data</div>}
        >
          <Show when={!loading()} fallback={<div class="empty-state">Loading...</div>}>
            {/* Only render the table when there are rows. Otherwise the <thead>
                would paint a lone actions-column strip (the "weird bar") when
                the table has no rows / no known fields, so show the empty state
                instead. */}
            <Show
              when={tableData().length > 0}
              fallback={<div class="empty-state">No data in table "{selectedTable()}"</div>}
            >
              <table class="data-table">
              <thead>
                <tr>
                  <For each={columns()}>{(column) => <th>{escapeHtml(column)}</th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={tableData()}>
                  {(row) => {
                    return (
                      <tr
                        classList={{ 'row-inspected': inspectedRow() === row }}
                        onClick={() => setInspectedRow(row)}
                      >
                        <For each={columns()}>
                          {(column) => (
                            <Cell
                              value={row[column]}
                              onIdClick={(id) => {
                                const parts = id.split(':');
                                const table = parts[0];
                                if (table && table !== selectedTable()) {
                                  setSelectedTable(table);
                                  props.setFilter(id);
                                } else {
                                  props.setFilter(id);
                                }
                              }}
                            />
                          )}
                        </For>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
            </Show>
          </Show>
        </Show>
      </div>

      <Show when={inspectedRow()}>
        <div class="row-pane">
          <div class="row-pane-head">
            <span class="row-pane-title" title={getRecordId(inspectedRow()!) ?? undefined}>
              {getRecordId(inspectedRow()!) ?? 'Row'}
            </span>
            <div class="row-pane-actions">
              <button
                class="icon-btn"
                onClick={copyInspected}
                title={copied() ? 'Copied' : 'Copy JSON'}
                aria-label="Copy JSON"
              >
                <Show when={copied()} fallback={<CopyIcon />}>
                  <CheckIcon />
                </Show>
              </button>
              <button
                class="icon-btn"
                title="Close"
                aria-label="Close"
                onClick={() => setInspectedRow(null)}
              >
                <CloseIcon />
              </button>
            </div>
          </div>
          <JsonView class="row-pane-json" value={inspectedRow()} />
        </div>
      </Show>

      <div class="db-toolbar">
        <button
          class="icon-btn"
          title="Refresh"
          aria-label="Refresh"
          onClick={() => setRefreshTick((t) => t + 1)}
        >
          <RefreshIcon />
        </button>
        <div class="db-toolbar-sep" />
        <input
          class="dt-filter-input db-toolbar-filter"
          type="text"
          placeholder="Filter"
          value={props.filter}
          onInput={(e) => props.setFilter(e.currentTarget.value)}
        />
        <select
          class="db-source-select"
          value={props.source}
          onChange={(e) => props.setSource(e.currentTarget.value as 'local' | 'remote')}
        >
          <option value="local">Local</option>
          <option value="remote">Remote</option>
        </select>
        <Show when={selectedTable() && !loading()}>
          <span class="db-toolbar-count">
            {tableData().length} {tableData().length === 1 ? 'row' : 'rows'}
          </span>
        </Show>
      </div>
    </div>
  );
}
