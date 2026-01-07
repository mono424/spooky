import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { useDevTools } from "../../context/DevToolsContext";
import { escapeHtml } from "../../utils/html";
import { Cell, type EditingCell } from "./Cell";

function getRecordId(row: Record<string, unknown>): string | null {
  if (!row.id) return null;
  if (typeof row.id === "string") return row.id;
  if (typeof row.id === "object" && row.id !== null) return row.id.toString();
  return String(row.id);
}

export function TableView() {
  const { state, selectedTable, setSelectedTable, fetchTableData, updateTableRow, deleteTableRow } = useDevTools();
  const [filter, setFilter] = createSignal("");
  // Track editing by Record ID and Column instead of Index
  const [editingCell, setEditingCell] = createSignal<EditingCell | null>(null);

  // Fetch table data when a table is selected
  createEffect(() => {
    const table = selectedTable();
    if (table) {
      if (typeof fetchTableData === 'function') {
         fetchTableData(table);
      }
      setFilter(""); // Reset filter when changing tables
    }
  });

  const tableData = createMemo(() => {
    const table = selectedTable();
    if (!table) return null;
    let data = state.database.tableData[table] || [];
    
    const filterText = filter().toLowerCase();
    if (filterText) {
      data = data.filter(row => {
        // Search in all values
        return Object.values(row).some(val => 
          String(val).toLowerCase().includes(filterText)
        );
      });
    }
    
    return data;
  });

  const columns = createMemo(() => {
    // Use original data for column discovery to ensure we see all potential columns even if filtered out
    const fullData = selectedTable() ? state.database.tableData[selectedTable()!] : [];
    
    if (!fullData || fullData.length === 0) return [];

    const keys = new Set<string>();
    fullData.forEach((row) => {
      Object.keys(row).forEach((key) => keys.add(key));
    });
    
    return Array.from(keys).sort((a, b) => {
      if (a.toLowerCase() === 'id') return -1;
      if (b.toLowerCase() === 'id') return 1;
      return a.localeCompare(b);
    });
  });

  const handleStartEdit = (recordId: string, column: string) => {
    setEditingCell({ recordId, column });
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
    if (!recordId) return;
    const tableName = selectedTable();
    if (!tableName) return;
    if (confirm(`Delete record ${recordId}?`)) {
      deleteTableRow(tableName, recordId);
    }
  };

  // Explicitly expose setFilter for external use if needed, but here we just bind
  return (
    <div class="database-data">
      <div class="table-controls" style={{ padding: "4px 8px", "border-bottom": "1px solid var(--sys-color-divider)", display: "flex", "align-items": "center" }}>
        <input
          type="text"
          placeholder="Filter..."
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
          style={{
            "width": "100%", /* Full width */
            "padding": "2px 12px", /* More horizontal padding for rounded pill */
            "height": "24px", 
            "background": "var(--sys-color-surface-container-highest, #3d3d3d)",
            "border": "1px solid var(--sys-color-outline-variant, #555)",
            "color": "var(--sys-color-on-surface, #fff)",
            "border-radius": "12px", /* Fully rounded (half of height) */
            "font-family": "var(--sys-typescale-body-font, '.SFNSDisplay-Regular', 'Helvetica Neue', 'Lucida Grande', sans-serif)",
            "font-size": "12px",
            "line-height": "16px",
            "outline": "none"
          }}
          onFocus={(e) => e.currentTarget.style.border = "1px solid var(--sys-color-primary, #1a73e8)"}
          onBlur={(e) => e.currentTarget.style.border = "1px solid var(--sys-color-outline-variant, #555)"}
        />
      </div>
      <div class="table-data">
        <Show
          when={selectedTable()}
          fallback={
            <div class="empty-state">Select a table to view data</div>
          }
        >
          <Show
            when={tableData() && tableData()!.length >= 0} 
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
                              recordId={recordId || ""}
                              isEditing={isEditingCell(column)}
                              onStartEdit={(col) => recordId && handleStartEdit(recordId, col)}
                              onUpdate={(newValue) => handleCellUpdate(row, column, newValue)}
                              onCancel={() => setEditingCell(null)}
                              onIdClick={(id) => {
                                // Expected format: "tableName:recordId"
                                const parts = id.split(':');
                                const table = parts[0];
                                if (table && table !== selectedTable()) {
                                  setSelectedTable(table);
                                  setFilter(id);
                                } else {
                                   setFilter(id);
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

