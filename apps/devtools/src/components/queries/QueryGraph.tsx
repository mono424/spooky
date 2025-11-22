import { For, Show, createMemo } from "solid-js";
import type { ActiveQuery } from "../../types/devtools";

interface QueryGraphProps {
  query: ActiveQuery;
  allQueries: ActiveQuery[];
}

export function QueryGraph(props: QueryGraphProps) {
  // Find queries that are connected to this query (listeners)
  const connectedQueries = createMemo(() => {
    const connected = props.query.connectedQueries || [];
    return props.allQueries.filter((q) => connected.includes(q.queryHash));
  });

  // Find queries that this query listens to (reverse lookup)
  const listeningTo = createMemo(() => {
    return props.allQueries.filter((q) =>
      q.connectedQueries?.includes(props.query.queryHash)
    );
  });

  const hasConnections = createMemo(() => {
    return (
      connectedQueries().length > 0 || listeningTo().length > 0
    );
  });

  return (
    <div class="query-graph">
      <div class="detail-section">
        <div class="detail-label">
          Listeners: {props.query.listenerCount ?? 0}
        </div>
      </div>

      <Show when={hasConnections()}>
        <div class="detail-section">
          <div class="detail-label">Query Connections</div>
          <div class="graph-container">
            {/* Center node - current query */}
            <div class="graph-node graph-node-center">
              <div class="graph-node-label">#{props.query.queryHash}</div>
            </div>

            {/* Queries this query listens to (incoming) */}
            <Show when={listeningTo().length > 0}>
              <div class="graph-group graph-group-incoming">
                <div class="graph-group-label">Listening To</div>
                <For each={listeningTo()}>
                  {(connectedQuery) => (
                    <div class="graph-node graph-node-incoming">
                      <div class="graph-node-label">
                        #{connectedQuery.queryHash}
                      </div>
                      <div class="graph-edge graph-edge-incoming"></div>
                    </div>
                  )}
                </For>
              </div>
            </Show>

            {/* Queries listening to this query (outgoing) */}
            <Show when={connectedQueries().length > 0}>
              <div class="graph-group graph-group-outgoing">
                <div class="graph-group-label">Listeners</div>
                <For each={connectedQueries()}>
                  {(connectedQuery) => (
                    <div class="graph-node graph-node-outgoing">
                      <div class="graph-edge graph-edge-outgoing"></div>
                      <div class="graph-node-label">
                        #{connectedQuery.queryHash}
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </div>
      </Show>

      <Show when={!hasConnections()}>
        <div class="detail-section">
          <div class="detail-value text-muted">
            No query connections
          </div>
        </div>
      </Show>
    </div>
  );
}

