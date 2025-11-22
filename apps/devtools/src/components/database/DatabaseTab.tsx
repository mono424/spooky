import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { useDevTools } from "../../context/DevToolsContext";
import { escapeHtml } from "../../utils/html";

function TableList() {
  const { state, selectedTable, setSelectedTable } = useDevTools();

  return (
    <div class="database-tables">
      <div class="database-header">
        <h2>Tables</h2>
      </div>
      <div class="tables-list">
        <Show
          when={state.database.tables.length > 0}
          fallback={<div class="empty-state">No tables available</div>}
        >
          <For each={state.database.tables}>
            {(table) => (
              <div
                class="table-item"
                classList={{ selected: selectedTable() === table }}
                onClick={() => setSelectedTable(table)}
              >
                {table}
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}

function TableView() {
  const { state, selectedTable, fetchTableData, updateTableRow, deleteTableRow } = useDevTools();
  const [editingCell, setEditingCell] = createSignal<{
    rowIndex: number;
    column: string;
  } | null>(null);
  const [editValue, setEditValue] = createSignal("");

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

  // Get record ID from a row
  const getRecordId = (row: Record<string, unknown>): string | null => {
    if (row.id && typeof row.id === "string") {
      return row.id;
    }
    return null;
  };

  // Handle cell click to start editing
  const handleCellClick = (e: MouseEvent, rowIndex: number, column: string, currentValue: unknown) => {
    // Don't allow editing the 'id' column
    if (column === "id") return;

    // Prevent event bubbling to avoid conflicts
    e.stopPropagation();

    setEditingCell({ rowIndex, column });
    const stringValue =
      currentValue !== undefined && currentValue !== null
        ? typeof currentValue === "object"
          ? JSON.stringify(currentValue)
          : String(currentValue)
        : "";
    setEditValue(stringValue);
  };

  // Handle cell value update
  const handleCellUpdate = (row: Record<string, unknown>, column: string) => {
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

    const newValue = editValue();
    const originalValue = row[column];

    // Convert original value to string for comparison
    const originalValueStr =
      originalValue !== undefined && originalValue !== null
        ? typeof originalValue === "object"
          ? JSON.stringify(originalValue)
          : String(originalValue)
        : "";

    // Don't update if value hasn't changed
    if (newValue === originalValueStr) {
      setEditingCell(null);
      return;
    }

    let parsedValue: unknown = newValue;

    // Try to parse JSON for objects
    if (newValue.startsWith("{") || newValue.startsWith("[")) {
      try {
        parsedValue = JSON.parse(newValue);
      } catch {
        // Keep as string if parsing fails
      }
    } else if (newValue === "true" || newValue === "false") {
      parsedValue = newValue === "true";
    } else if (!isNaN(Number(newValue)) && newValue !== "") {
      parsedValue = Number(newValue);
    }

    updateTableRow(tableName, recordId, { [column]: parsedValue });
    setEditingCell(null);
  };

  // Handle delete row
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

  // Handle key down in edit input
  const handleKeyDown = (e: KeyboardEvent, row: Record<string, unknown>, column: string) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      handleCellUpdate(row, column);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setEditingCell(null);
    }
  };

  // Handle blur to save changes
  const handleBlur = (row: Record<string, unknown>, column: string, rowIndex: number) => {
    // Use setTimeout to allow click events to process first
    setTimeout(() => {
      const editing = editingCell();
      // Only update if we're still editing this cell
      if (editing && editing.rowIndex === rowIndex && editing.column === column) {
        handleCellUpdate(row, column);
      }
    }, 200);
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
                  {(row, rowIndex) => (
                    <tr>
                      <For each={columns()}>
                        {(column) => {
                          const editing = editingCell();
                          const isEditing =
                            editing !== null &&
                            editing.rowIndex === rowIndex() &&
                            editing.column === column;
                          const cellValue = row[column];
                          const displayValue =
                            cellValue !== undefined && cellValue !== null
                              ? typeof cellValue === "object"
                                ? JSON.stringify(cellValue)
                                : String(cellValue)
                              : "";

                          return (
                            <td
                              class="editable-cell"
                              classList={{ editing: isEditing, readonly: column === "id" }}
                              onClick={(e) =>
                                !isEditing &&
                                handleCellClick(e, rowIndex(), column, cellValue)
                              }
                            >
                              <Show when={isEditing} fallback={displayValue}>
                                <input
                                  ref={(el) => {
                                    // Focus and select when input is rendered
                                    if (el) {
                                      setTimeout(() => {
                                        el.focus();
                                        el.select();
                                      }, 0);
                                    }
                                  }}
                                  type="text"
                                  class="cell-input"
                                  value={editValue()}
                                  onInput={(e) => setEditValue(e.currentTarget.value)}
                                  onBlur={() => handleBlur(row, column, rowIndex())}
                                  onKeyDown={(e) => handleKeyDown(e, row, column)}
                                  onClick={(e) => e.stopPropagation()}
                                />
                              </Show>
                            </td>
                          );
                        }}
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
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>
    </div>
  );
}

export function DatabaseTab() {
  return (
    <div class="database-container">
      <TableList />
      <TableView />
    </div>
  );
}
