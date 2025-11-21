import { For, Show, createEffect, createMemo } from "solid-js";
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
  const { state, selectedTable, fetchTableData } = useDevTools();

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
                </tr>
              </thead>
              <tbody>
                <For each={tableData()}>
                  {(row) => (
                    <tr>
                      <For each={columns()}>
                        {(column) => (
                          <td>
                            {row[column] !== undefined && row[column] !== null
                              ? typeof row[column] === "object"
                                ? JSON.stringify(row[column])
                                : String(row[column])
                              : ""}
                          </td>
                        )}
                      </For>
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
