import { TableList } from "./TableList";
import { TableView } from "./TableView";

export function DatabaseTab() {
  return (
    <div class="database-container">
      <TableList />
      <TableView />
    </div>
  );
}
