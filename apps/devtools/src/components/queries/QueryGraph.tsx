import { Show } from 'solid-js';
import type { ActiveQuery } from '../../types/devtools';

interface QueryGraphProps {
  query: ActiveQuery;
  allQueries: ActiveQuery[];
}

export function QueryGraph(props: QueryGraphProps) {
  // We don't need connection logic anymore as we are displaying data directly

  return (
    <div class="query-graph">
      <div class="detail-section">
        <div class="detail-label">Query Result Data</div>
        <div class="detail-value">
          <Show when={props.query.data} fallback={<div class="text-muted">No data available</div>}>
            <pre class="query-code">{JSON.stringify(props.query.data, null, 2)}</pre>
          </Show>
        </div>
      </div>
    </div>
  );
}
