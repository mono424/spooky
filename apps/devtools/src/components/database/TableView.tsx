import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { useDevTools } from "../../context/DevToolsContext";
import { escapeHtml } from "../../utils/html";
import { Cell, type EditingCell } from "./Cell";

function getRecordId(row: Record<string, unknown>): string | null {
  if (row.id && typeof row.id === "string") {
    return row.id;
  }
  return null;
}

export function TableView() {
  const { state, selectedTable, fetchTableData, updateTableRow, deleteTableRow } = useDevTools();
  const [editingCell, setEditingCell] = createSignal<EditingCell | null>(null);

  // Fetch table data when a table is selected
  createEffect(() => {
    const table = selectedTable();
    if (table) {
      fetchTableData(table);
    }
  });

  const tableData = createMemo(() => {
    const table = selectedTable();
    if (!table) return null;
    return state.database.tableData[table] || [];
  });

  const columns = createMemo(() => {
    const data = tableData();
    if (!data || data.length === 0) return [];

    // Get all unique keys from all rows
    const keys = new Set<string>();
    data.forEach((row) => {
      Object.keys(row).forEach((key) => keys.add(key));
    });
    return Array.from(keys);
  });

  const handleStartEdit = (rowIndex: number, column: string) => {
    setEditingCell({ rowIndex, column });
  };

  const handleCellUpdate = (row: Record<string, unknown>, column: string, newValue: unknown) => {
    const editing = editingCell();
    if (!editing) return;

    const recordId = getRecordId(row);
    if (!recordId) {
      console.error("Cannot update row: no id found");
      setEditingCell(null);
      return;
    }

    const tableName = selectedTable();
    if (!tableName) {
      setEditingCell(null);
      return;
    }

    const originalValue = row[column];

    // Convert original value to string for comparison
    const originalValueStr =
      originalValue !== undefined && originalValue !== null
        ? typeof originalValue === "object"
          ? JSON.stringify(originalValue)
          : String(originalValue)
        : "";

    const newValueStr =
      newValue !== undefined && newValue !== null
        ? typeof newValue === "object"
          ? JSON.stringify(newValue)
          : String(newValue)
        : "";

    // Don't update if value hasn't changed
    if (newValueStr === originalValueStr) {
      setEditingCell(null);
      return;
    }

    updateTableRow(tableName, recordId, { [column]: newValue });
    setEditingCell(null);
  };

  const handleDeleteRow = (row: Record<string, unknown>) => {
    const recordId = getRecordId(row);
    if (!recordId) {
      console.error("Cannot delete row: no id found");
      return;
    }

    const tableName = selectedTable();
    if (!tableName) return;

    if (confirm(`Delete record ${recordId}?`)) {
      deleteTableRow(tableName, recordId);
    }
  };

  return (
    <div class="database-data">
      <div class="table-data">
        <Show
          when={selectedTable()}
          fallback={
            <div class="empty-state">Select a table to view data</div>
          }
        >
          <Show
            when={tableData() && tableData()!.length > 0}
            fallback={
              <div class="empty-state">
                No data in table "{selectedTable()}"
              </div>
            }
          >
            <table class="data-table">
              <thead>
                <tr>
                  <For each={columns()}>
                    {(column) => <th>{escapeHtml(column)}</th>}
                  </For>
                  <th class="delete-column">Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={tableData()}>
                  {(row, rowIndex) => {
                    const editing = editingCell();
                    const isEditingCell = (column: string) =>
                      editing !== null &&
                      editing.rowIndex === rowIndex() &&
                      editing.column === column;

                    return (
                      <tr>
                        <For each={columns()}>
                          {(column) => (
                            <Cell
                              value={row[column]}
                              column={column}
                              rowIndex={rowIndex()}
                              isEditing={isEditingCell(column)}
                              onStartEdit={handleStartEdit}
                              onUpdate={(newValue) => handleCellUpdate(row, column, newValue)}
                              onCancel={() => setEditingCell(null)}
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

