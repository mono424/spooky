import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { escapeHtml } from '../../utils/html';
import { Cell } from './Cell';
import { JsonView } from '../ui/JsonView';

function getRecordId(row: Record<string, unknown>): string | null {
  if (!row.id) return null;
  if (typeof row.id === 'string') return row.id;
  if (typeof row.id === 'object' && row.id !== null) return row.id.toString();
  return String(row.id);
}

function CopyIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="9" y="9" width="12" height="12" rx="2" ry="2"></rect>
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="20 6 9 17 4 12"></polyline>
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
      <line x1="6" y1="6" x2="18" y2="18"></line>
      <line x1="18" y1="6" x2="6" y2="18"></line>
    </svg>
  );
}

function ChevronLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="15 18 9 12 15 6"></polyline>
    </svg>
  );
}

function ChevronRightIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="9 18 15 12 9 6"></polyline>
    </svg>
  );
}

const PAGE_SIZES = [10, 20, 50, 100];

/**
 * The SQLite local engine speaks a bounded SurrealQL subset (see core's
 * `surql-translate.ts`), so a statement it can't translate fails with a message
 * that reads like a client bug. Say what it actually means, and where the same
 * query does work.
 */
function explainQueryError(msg: string, source: 'local' | 'remote'): string {
  if (source === 'local' && msg.includes('unsupported SurrealQL for translation')) {
    return `${msg} — the local store is SQLite, which understands only a subset of SurrealQL. Switch the source to Remote to run this query.`;
  }
  return msg;
}

/**
 * SurrealDB can return different formats depending on version and transport:
 * [{ status: 'OK', result: [...] }], [[...]], a bare array of records, or a
 * single { result: ... } object. Normalize all of them to a plain array.
 */
function unwrapQueryResult(result: unknown): unknown[] {
  if (Array.isArray(result)) {
    if (
      result.length > 0 &&
      result[0] &&
      typeof result[0] === 'object' &&
      'result' in result[0]
    ) {
      const inner = (result[0] as { result: unknown }).result;
      return Array.isArray(inner) ? inner : [];
    }
    if (result.length > 0 && Array.isArray(result[0])) {
      return result[0];
    }
    return result;
  }
  if (result && typeof result === 'object' && 'result' in result) {
    const inner = (result as { result: unknown }).result;
    return Array.isArray(inner) ? inner : [];
  }
  return [];
}

interface TableViewProps {
  filter: string;
  setFilter: (val: string) => void;
  source: 'local' | 'remote';
  setSource: (val: 'local' | 'remote') => void;
  onError?: (msg: string) => void;
}

export function TableView(props: TableViewProps) {
  const { state, selectedTable, setSelectedTable, runQuery, dbRefreshNonce, isFetchingRows, setFetchingRows } =
    useDevTools();

  // 'surreal' | 'sqlite' | 'custom'; absent against a page whose core predates
  // the field, in which case the picker just reads "Local" as before.
  const localEngine = () => state.database.engine;
  // Filter and source are now props

  // Row inspected in the bottom JSON pane
  const [inspectedRow, setInspectedRow] = createSignal<Record<string, unknown> | null>(null);
  const [copied, setCopied] = createSignal(false);

  const copyInspected = async () => {
    const row = inspectedRow();
    if (!row) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(row, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error('[TableView] Copy failed:', err);
    }
  };

  const [page, setPage] = createSignal(1);
  const [limit, setLimit] = createSignal(20);
  // Total row count in the table (from a separate COUNT query), null while unknown
  const [total, setTotal] = createSignal<number | null>(null);

  // Back to the first page whenever the table, source, or page size changes
  createEffect(
    on([selectedTable, () => props.source, limit], () => setPage(1), { defer: true })
  );

  // Monotonic id for the in-flight fetch. Re-running the effect (refresh, paging,
  // switching table/source) starts a new batch while the previous one may still
  // be resolving, and the two are NOT ordered: without this guard a slow earlier
  // batch can land after a fast later one and overwrite fresh rows with stale
  // ones, and its `allSettled` clears the toolbar spinner while the current
  // fetch is still running. Stale batches drop their results on the floor.
  let fetchGeneration = 0;

  // Fetch table data when a table is selected, source/page/limit changes, or refresh clicked
  createEffect(() => {
    const table = selectedTable();
    const currentSource = props.source;
    const currentLimit = limit();
    const currentPage = page();
    // Read before the `if (table && ...)` guard below so the effect stays
    // subscribed to the toolbar Refresh even with no table selected.
    dbRefreshNonce();
    setInspectedRow(null); // close the JSON pane when the table/source changes
    if (table && runQuery) {
      const run = ++fetchGeneration;
      const isStale = () => run !== fetchGeneration;
      setFetchingRows(true);
      const start = (currentPage - 1) * currentLimit;
      console.log('[TableView] Triggering query for table:', table, 'Source:', currentSource);
      const dataPromise = runQuery(
        `SELECT * FROM ${table} LIMIT ${currentLimit} START ${start}`,
        currentSource
      )
        .then((result: any) => {
          if (isStale()) return undefined;
          console.log('[TableView] Query result:', result);
          setData(unwrapQueryResult(result) as Record<string, unknown>[]);
          return undefined;
        })
        .catch((err) => {
          if (isStale()) return;
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
          props.onError?.(explainQueryError(msg, currentSource));
          setData([]);
        });
      const countPromise = runQuery(`SELECT count() FROM ${table} GROUP ALL`, currentSource)
        .then((result: any) => {
          if (isStale()) return undefined;
          const rows = unwrapQueryResult(result) as Array<{ count?: unknown }>;
          const count = rows[0]?.count;
          setTotal(typeof count === 'number' ? count : null);
          return undefined;
        })
        .catch((err) => {
          if (isStale()) return;
          console.error('[TableView] Count Query Error:', err);
          setTotal(null);
        });
      Promise.allSettled([dataPromise, countPromise]).then(() => {
        if (!isStale()) setFetchingRows(false);
        return undefined;
      });
    }
  });

  const [data, setData] = createSignal<Record<string, unknown>[]>([]);
  // Unmounting mid-query would otherwise leave the toolbar spinner running.
  onCleanup(() => setFetchingRows(false));

  const totalPages = createMemo(() => {
    const t = total();
    if (t === null) return null;
    return Math.max(1, Math.ceil(t / limit()));
  });

  // Clamp the page if the table shrank underneath us (e.g. rows deleted, then refresh)
  createEffect(() => {
    const pages = totalPages();
    if (pages !== null && page() > pages) setPage(pages);
  });

  const pageOptions = createMemo(() => {
    const count = totalPages() ?? page();
    return Array.from({ length: count }, (_, i) => i + 1);
  });

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

    const finalCols = Array.from(allKeys).toSorted((a, b) => {
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

  return (
    <div class="database-data">
      <div class="table-data">
        <Show
          when={selectedTable()}
          fallback={<div class="empty-state">Select a table to view data</div>}
        >
          <Show when={!isFetchingRows()} fallback={<div class="empty-state">Loading...</div>}>
            {/* Only render the table when there are rows. Otherwise the <thead>
                would paint a lone actions-column strip (the "weird bar") when
                the table has no rows / no known fields, so show the empty state
                instead. */}
            <Show
              when={tableData().length > 0}
              fallback={<div class="empty-state">No data in table "{selectedTable()}"</div>}
            >
              <table class="data-table">
              <thead>
                <tr>
                  <For each={columns()}>{(column) => <th>{escapeHtml(column)}</th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={tableData()}>
                  {(row) => {
                    return (
                      <tr
                        classList={{ 'row-inspected': inspectedRow() === row }}
                        onClick={() => setInspectedRow(row)}
                      >
                        <For each={columns()}>
                          {(column) => (
                            <Cell
                              value={row[column]}
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
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
            </Show>
          </Show>
        </Show>
      </div>

      <Show when={inspectedRow()}>
        <div class="row-pane">
          <div class="row-pane-head">
            <span class="row-pane-title" title={getRecordId(inspectedRow()!) ?? undefined}>
              {getRecordId(inspectedRow()!) ?? 'Row'}
            </span>
            <div class="row-pane-actions">
              <button
                class="icon-btn"
                onClick={copyInspected}
                title={copied() ? 'Copied' : 'Copy JSON'}
                aria-label="Copy JSON"
              >
                <Show when={copied()} fallback={<CopyIcon />}>
                  <CheckIcon />
                </Show>
              </button>
              <button
                class="icon-btn"
                title="Close"
                aria-label="Close"
                onClick={() => setInspectedRow(null)}
              >
                <CloseIcon />
              </button>
            </div>
          </div>
          <JsonView class="row-pane-json" value={inspectedRow()} />
        </div>
      </Show>

      {/* Refresh lives in the top toolbar now — it re-runs this view's fetch
          via `dbRefreshNonce`. */}
      <div class="db-toolbar">
        <input
          class="dt-filter-input db-toolbar-filter"
          type="text"
          placeholder="Filter"
          value={props.filter}
          onInput={(e) => props.setFilter(e.currentTarget.value)}
        />
        <select
          class="db-source-select"
          title={`Local store: ${localEngine() ?? 'unknown engine'}`}
          value={props.source}
          onChange={(e) => props.setSource(e.currentTarget.value as 'local' | 'remote')}
        >
          {/* Naming the engine matters here: SQLite and SurrealDB-WASM answer
              the same query differently (bounded SurrealQL, link fields as
              strings), so "Local" alone hides which one you are looking at. */}
          <option value="local">{localEngine() ? `Local (${localEngine()})` : 'Local'}</option>
          <option value="remote">Remote</option>
        </select>
        <Show when={selectedTable()}>
          <div class="db-toolbar-right">
            <select
              class="db-source-select"
              title="Rows per page"
              aria-label="Rows per page"
              value={String(limit())}
              onChange={(e) => setLimit(Number(e.currentTarget.value))}
            >
              <For each={PAGE_SIZES}>
                {(size) => <option value={String(size)}>{size} / page</option>}
              </For>
            </select>
            <div class="db-toolbar-sep" />
            <button
              class="icon-btn"
              title="Previous page"
              aria-label="Previous page"
              disabled={page() <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
            >
              <ChevronLeftIcon />
            </button>
            <select
              class="db-source-select"
              title="Page"
              aria-label="Page"
              value={String(page())}
              onChange={(e) => setPage(Number(e.currentTarget.value))}
            >
              <For each={pageOptions()}>{(n) => <option value={String(n)}>{n}</option>}</For>
            </select>
            <span class="db-toolbar-pages">/ {totalPages() ?? '?'}</span>
            <button
              class="icon-btn"
              title="Next page"
              aria-label="Next page"
              disabled={totalPages() !== null && page() >= totalPages()!}
              onClick={() => setPage((p) => p + 1)}
            >
              <ChevronRightIcon />
            </button>
            <div class="db-toolbar-sep" />
            <Show when={!isFetchingRows()}>
              <span class="db-toolbar-count">
                <Show
                  when={total() !== null}
                  fallback={`${tableData().length} ${tableData().length === 1 ? 'row' : 'rows'}`}
                >
                  <Show when={props.filter} fallback={`${total()} ${total() === 1 ? 'item' : 'items'}`}>
                    {tableData().length} of {total()} items
                  </Show>
                </Show>
              </span>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
