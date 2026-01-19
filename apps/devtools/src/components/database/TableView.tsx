import { For, Show, createEffect, createMemo, createSignal } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { escapeHtml } from '../../utils/html';
import { Cell, type EditingCell } from './Cell';

function getRecordId(row: Record<string, unknown>): string | null {
  if (!row.id) return null;
  if (typeof row.id === 'string') return row.id;
  if (typeof row.id === 'object' && row.id !== null) return row.id.toString();
  return String(row.id);
}

interface TableViewProps {
  filter: string;
  setFilter: (val: string) => void;
  source: 'local' | 'remote';
  onError?: (msg: string) => void;
}

export function TableView(props: TableViewProps) {
  const { selectedTable, setSelectedTable, runQuery, updateTableRow, deleteTableRow } =
    useDevTools();
  // Filter and source are now props

  // Track editing by Record ID and Column instead of Index
  const [editingCell, setEditingCell] = createSignal<EditingCell | null>(null);

  // Fetch table data when a table is selected or source changes
  createEffect(() => {
    const table = selectedTable();
    const currentSource = props.source;
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

    const finalCols = Array.from(allKeys).sort((a, b) => {
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

  const handleStartEdit = (recordId: string, column: string) => {
    setEditingCell({ recordId, column });
  };

  const handleCellUpdate = (row: Record<string, unknown>, column: string, newValue: unknown) => {
    const editing = editingCell();
    if (!editing) return;

    const recordId = getRecordId(row);
    if (!recordId) {
      console.error('Cannot update row: no id found');
      setEditingCell(null);
      return;
    }

    const tableName = selectedTable();
    if (!tableName) {
      setEditingCell(null);
      return;
    }

    const originalValue = row[column];
    const originalValueStr =
      originalValue !== undefined && originalValue !== null
        ? typeof originalValue === 'object'
          ? JSON.stringify(originalValue)
          : String(originalValue)
        : '';

    const newValueStr =
      newValue !== undefined && newValue !== null
        ? typeof newValue === 'object'
          ? JSON.stringify(newValue)
          : String(newValue)
        : '';

    if (newValueStr === originalValueStr) {
      setEditingCell(null);
      return;
    }

    updateTableRow(tableName, recordId, { [column]: newValue });
    setEditingCell(null);

    // Refresh data after update
    const table = selectedTable();
    const currentSource = props.source;
    if (table && runQuery) {
      runQuery(`SELECT * FROM ${table} LIMIT 20`, currentSource)
        .then((res: any) => {
          // Re-use logic or simple set for now, ideally refactor the resolver function
          if (Array.isArray(res)) {
            if (res.length > 0 && res[0] && typeof res[0] === 'object' && 'result' in res[0]) {
              const queryResult = res[0].result;
              setData(Array.isArray(queryResult) ? queryResult : []);
            } else {
              setData(res);
            }
          }
        })
        .catch(console.error);
    }
  };

  const handleDeleteRow = (row: Record<string, unknown>) => {
    const recordId = getRecordId(row);
    if (!recordId) return;
    const tableName = selectedTable();
    if (!tableName) return;
    if (confirm(`Delete record ${recordId}?`)) {
      deleteTableRow(tableName, recordId);
      // Refresh data after delete
      const table = selectedTable();
      const currentSource = props.source;
      if (table && runQuery) {
        runQuery(`SELECT * FROM ${table} LIMIT 20`, currentSource)
          .then((res: any) => {
            if (Array.isArray(res)) {
              if (res.length > 0 && res[0] && typeof res[0] === 'object' && 'result' in res[0]) {
                const queryResult = res[0].result;
                setData(Array.isArray(queryResult) ? queryResult : []);
              } else {
                setData(res);
              }
            }
          })
          .catch(console.error);
      }
    }
  };

  return (
    <div class="database-data">
      <div class="table-data">
        <Show
          when={selectedTable()}
          fallback={<div class="empty-state">Select a table to view data</div>}
        >
          <Show
            when={!loading() && tableData() && tableData()!.length >= 0}
            fallback={
              loading() ? (
                <div class="empty-state">Loading...</div>
              ) : (
                <div class="empty-state">No data in table "{selectedTable()}"</div>
              )
            }
          >
            <table class="data-table">
              <thead>
                <tr>
                  <For each={columns()}>{(column) => <th>{escapeHtml(column)}</th>}</For>
                  <th class="delete-column">Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={tableData()}>
                  {(row) => {
                    const recordId = getRecordId(row);
                    const editing = editingCell();

                    const isEditingCell = (column: string) =>
                      editing !== null &&
                      recordId !== null &&
                      editing.recordId === recordId &&
                      editing.column === column;

                    return (
                      <tr>
                        <For each={columns()}>
                          {(column) => (
                            <Cell
                              value={row[column]}
                              column={column}
                              recordId={recordId || ''}
                              isEditing={isEditingCell(column)}
                              onStartEdit={(col) => recordId && handleStartEdit(recordId, col)}
                              onUpdate={(newValue) => handleCellUpdate(row, column, newValue)}
                              onCancel={() => setEditingCell(null)}
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
                        <td class="delete-cell">
                          <button
                            class="delete-btn"
                            onClick={() => handleDeleteRow(row)}
                            title="Delete row"
                          >
                            ×
                          </button>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>
    </div>
  );
}
