import { For, Show } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';

export function TableList() {
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
