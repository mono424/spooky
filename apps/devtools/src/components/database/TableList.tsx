import { For, Show, createMemo } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import type { DatabaseState } from '../../types/devtools';

interface TableListProps {
  /** Tables for the currently-selected source (local or remote). */
  tables: string[];
  /** When false, `_00_*` internal sync tables are hidden. */
  showInternal: boolean;
  onToggleInternal: (value: boolean) => void;
  /** Durability of the local store, when the engine reports it. */
  storage?: DatabaseState['storage'];
  /** Shared-tabs role state, when the feature is active. */
  tabs?: DatabaseState['tabs'];
}

/** Internal sync/bookkeeping tables the app doesn't normally care about. */
const INTERNAL_RE = /^_00_/;

function EyeIcon(props: { off: boolean }) {
  return (
    <Show
      when={props.off}
      fallback={
        <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
          <path d="M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5C21.27 7.61 17 4.5 12 4.5zm0 12a4.5 4.5 0 1 1 0-9 4.5 4.5 0 0 1 0 9zm0-7a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z" />
        </svg>
      }
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
        <path d="M12 6.5c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92A11.8 11.8 0 0 0 23 11.5C21.27 7.11 17 4 12 4c-1.4 0-2.74.25-3.98.7l2.16 2.16c.57-.23 1.18-.36 1.82-.36zM2.71 3.16 1.29 4.58l1.97 1.97A11.79 11.79 0 0 0 1 11.5C2.73 15.89 7 19 12 19c1.52 0 2.98-.29 4.32-.82l2.68 2.68 1.41-1.41L2.71 3.16zM12 16.5c-2.76 0-5-2.24-5-5 0-.77.18-1.5.49-2.14l1.57 1.57c-.03.19-.06.37-.06.57a2.99 2.99 0 0 0 3 3c.2 0 .38-.03.57-.06l1.57 1.57c-.65.31-1.37.49-2.14.49z" />
      </svg>
    </Show>
  );
}

/** Small "table" glyph shown before every table name. */
function TableIcon() {
  return (
    <svg
      class="table-item-icon"
      viewBox="0 0 24 24"
      width="13"
      height="13"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M4 4h16a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm1 4v3h6V8H5zm8 0v3h6V8h-6zm-8 5v3h6v-3H5zm8 0v3h6v-3h-6z" />
    </svg>
  );
}

/** Crossed-eye glyph marking a normally-hidden internal table. */
function EyeOffBadge() {
  return (
    <svg
      class="table-item-hidden"
      viewBox="0 0 24 24"
      width="12"
      height="12"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M12 6.5c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92A11.8 11.8 0 0 0 23 11.5C21.27 7.11 17 4 12 4c-1.4 0-2.74.25-3.98.7l2.16 2.16c.57-.23 1.18-.36 1.82-.36zM2.71 3.16 1.29 4.58l1.97 1.97A11.79 11.79 0 0 0 1 11.5C2.73 15.89 7 19 12 19c1.52 0 2.98-.29 4.32-.82l2.68 2.68 1.41-1.41L2.71 3.16zM12 16.5c-2.76 0-5-2.24-5-5 0-.77.18-1.5.49-2.14l1.57 1.57c-.03.19-.06.37-.06.57a2.99 2.99 0 0 0 3 3c.2 0 .38-.03.57-.06l1.57 1.57c-.65.31-1.37.49-2.14.49z" />
    </svg>
  );
}

export function TableList(props: TableListProps) {
  const { selectedTable, setSelectedTable } = useDevTools();

  const tables = createMemo(() =>
    props.showInternal ? props.tables : props.tables.filter((t) => !INTERNAL_RE.test(t))
  );

  // The page-side serializer renders a missing value as the STRING 'undefined',
  // so an absent reason must not turn into a tooltip.
  const storageError = () => {
    const err = props.storage?.error;
    return err && err !== 'undefined' ? err : undefined;
  };

  /** ` · leader` / ` · follower of a1b2c3d4`, or nothing outside shared-tabs. */
  const roleSuffix = () => {
    const t = props.tabs;
    if (!t?.active || !t.role) return '';
    if (t.role === 'follower' && t.leaderTabId) {
      return ` · follower of ${t.leaderTabId.slice(0, 8)}`;
    }
    return ` · ${t.role}`;
  };

  return (
    <div class="database-tables">
      <div class="database-header">
        <h2>Tables</h2>
        <button
          class="icon-btn database-header-toggle"
          classList={{ active: props.showInternal }}
          title={props.showInternal ? 'Hide internal (_00_) tables' : 'Show internal (_00_) tables'}
          aria-label="Toggle internal tables"
          aria-pressed={props.showInternal}
          onClick={() => props.onToggleInternal(!props.showInternal)}
        >
          <EyeIcon off={!props.showInternal} />
        </button>
      </div>
      <div class="tables-list">
        <Show
          when={tables().length > 0}
          fallback={<div class="empty-state">No tables available</div>}
        >
          <For each={tables()}>
            {(table) => (
              <div
                class="table-item"
                classList={{ selected: selectedTable() === table }}
                onClick={() => setSelectedTable(table)}
                title={table}
              >
                <TableIcon />
                <span class="table-item-name">{table}</span>
                <Show when={INTERNAL_RE.test(table)}>
                  <EyeOffBadge />
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
      {/* Durability of the local store. A store that fell back to memory holds
          the whole dataset in RAM and loses local writes on reload, which is
          otherwise invisible from the outside. */}
      <Show when={props.storage && props.storage.status !== 'unknown'}>
        <div
          class="database-storage"
          classList={{ fallback: props.storage!.fallback }}
          title={storageError()}
        >
          Storage: {props.storage!.status}
          {props.storage!.fallback ? ' (fallback, not persisted)' : ''}
          {/* Shared-tabs: whose store these tables actually live in. Details
              (term, followers, relay counters) are in the Storage tab. */}
          {roleSuffix()}
        </div>
      </Show>
    </div>
  );
}
