import {
  createContext,
  useContext,
  createSignal,
  onMount,
  type ParentComponent,
} from 'solid-js';
import { createStore } from 'solid-js/store';
import {
  DEFAULT_VERSIONS,
  type DevToolsState,
  type BackendDevToolsState,
  type TabType,
  type ChromeMessage,
} from '../types/devtools';
import { useChromeConnection } from '../hooks/useChromeConnection';
import { useRunInHostPage } from '../hooks/useRunInHostPage';
import { adaptBackendState } from '../utils/state-adapter';

interface McpStatus {
  enabled: boolean;
  connected: boolean;
  port: number;
}

interface DevToolsContextValue {
  // State
  state: DevToolsState;
  activeTab: () => TabType;
  selectedQueryHash: () => number | null;
  selectedTable: () => string | null;
  isSp00kyAvailable: () => boolean;
  mcpStatus: () => McpStatus;
  setMcpEnabled: (enabled: boolean) => void;

  // Actions
  setActiveTab: (tab: TabType) => void;
  setSelectedQueryHash: (hash: number | null) => void;
  setSelectedTable: (table: string | null) => void;
  clearEvents: () => void;
  refresh: () => void;
  refreshVersions: () => void;
  fetchTableData: (tableName: string) => void;
  updateTableRow: (tableName: string, recordId: string, updates: Record<string, unknown>) => void;
  deleteTableRow: (tableName: string, recordId: string) => void;
  runQuery?: (query: string, target: 'local' | 'remote') => Promise<any>;
  fetchSchema?: () => Promise<void>;
  fetchTables?: (target?: 'local' | 'remote') => Promise<void>;
}

/** Union of two table-name lists, preserving `a`'s order then appending new `b`. */
function unionTables(a: string[], b: string[]): string[] {
  const seen = new Set(a);
  const extra = b.filter((t) => !seen.has(t));
  return extra.length === 0 ? a : [...a, ...extra];
}

const DevToolsContext = createContext<DevToolsContextValue>();

export const DevToolsProvider: ParentComponent = (props) => {
  // Store for DevTools state
  // oxlint-disable-next-line no-shadow -- intentionally matching interface field name
  const [state, setState] = createStore<DevToolsState>({
    events: [],
    activeQueries: [],
    auth: {
      isAuthenticated: false,
      user: null,
      lastAuthCheck: Date.now(),
    },
    database: {
      tables: [],
      remoteTables: [],
      tableData: {},
    },
    versions: DEFAULT_VERSIONS,
  });

  // UI state
  const [activeTab, setActiveTab] = createSignal<TabType>('queries');
  const [selectedQueryHash, setSelectedQueryHash] = createSignal<number | null>(null);
  const [selectedTable, setSelectedTable] = createSignal<string | null>(null);
  const [isSp00kyAvailable, setIsSp00kyAvailable] = createSignal(false);
  const [mcpStatus, setMcpStatus] = createSignal<McpStatus>({ enabled: false, connected: false, port: 9315 });

  // Custom hooks
  const { requestState, sendMessage } = useChromeConnection({
    onMessage: handleMessage,
    onConnect: () => {
      console.log('[DevTools] Chrome connection established');
      checkSp00ky();
      sendMessage({ type: 'GET_MCP_STATUS' });
    },
    onDisconnect: () => {
      console.log('[DevTools] Chrome connection lost');
      setIsSp00kyAvailable(false);
    },
  });

  const hostPage = useRunInHostPage();

  // In-flight guards for the panel's own optional schema queries (kept for
  // manual/debug use). The table list itself comes from the backend now.
  let tablesInFlight = false;
  let remoteTablesInFlight = false;
  let schemaInFlight = false;

  /**
   * Handle messages from background script
   */
  // Stores pending query requests: requestId -> { resolve, reject }
  const pendingQueries = new Map<
    string,
    { resolve: (data: any) => void; reject: (err: string) => void }
  >();

  /**
   * Handle messages from background script
   */
  function handleMessage(message: ChromeMessage) {
    console.log('[DevTools] Processing message:', message);

    switch (message.type) {
      case 'SP00KY_DETECTED':
        setIsSp00kyAvailable(true);
        // If state is included in the detection message, use it
        if (message.data && (message.data as any).state) {
          console.log('[DevToolsContext] Initialized with state from detection');
          updateState((message.data as any).state);
        } else {
          console.log('[DevToolsContext] Sp00ky detected, requesting state...');
          requestState();
        }
        break;

      case 'SP00KY_STATE_CHANGED':
        if (message.state) {
          console.log(
            '[DevToolsContext] State updated. Tables:',
            message.state.database?.tables?.length || 0
          );
          updateState(message.state);
        }
        break;

      case 'SP00KY_TABLE_DATA_RESPONSE':
        if (message.tableName && message.data) {
          setState(
            'database',
            'tableData',
            message.tableName,
            message.data as Record<string, unknown>[]
          );
        }
        break;

      case 'SP00KY_QUERY_RESPONSE':
        const msg = message as any;
        console.log('[DevTools] RAW QUERY RESPONSE:', msg);

        if (msg.requestId && pendingQueries.has(msg.requestId)) {
          // oxlint-disable-next-line no-non-null-assertion -- guarded by .has() check above
          const { resolve, reject } = pendingQueries.get(msg.requestId)!;
          pendingQueries.delete(msg.requestId);

          if (msg.success) {
            resolve(msg.data);
          } else {
            console.error('[DevTools] Rejecting with error:', msg.error);
            reject(msg.error || 'Unknown error from query response (msg.error was falsy)');
          }
        } else {
          console.warn(
            '[DevTools] Received query response for unknown/expired requestId:',
            msg.requestId
          );
        }
        break;

      case 'MCP_STATUS':
        setMcpStatus({
          enabled: (message as any).enabled ?? false,
          connected: (message as any).connected ?? false,
          port: (message as any).port ?? 9315,
        });
        break;

      case 'PAGE_RELOADED':
        console.log('[DevTools] Page reloaded, checking for Sp00ky...');
        setTimeout(() => {
          checkSp00ky();
        }, 500);
        // Clear pending queries on reload
        pendingQueries.forEach(({ reject }) => reject('Page reloaded'));
        pendingQueries.clear();
        break;

      default:
        console.log('[DevTools] Unknown message type:', message.type);
    }
  }

  /**
   * Update state from Sp00ky - accepts backend state format
   */
  function updateState(backendState: BackendDevToolsState | DevToolsState) {
    console.log('[DevTools] Received state:', backendState);

    // Check if it's backend format (has eventsHistory) or frontend format (has events)
    const frontendState =
      'eventsHistory' in backendState
        ? adaptBackendState(backendState as BackendDevToolsState)
        : (backendState as DevToolsState);

    console.log('[DevTools] Adapted state:', frontendState);

    // Update events
    if (frontendState.events) {
      setState('events', frontendState.events);
    }

    // Update active queries
    if (frontendState.activeQueries) {
      setState('activeQueries', frontendState.activeQueries);
    }

    // Update auth
    if (frontendState.auth) {
      setState('auth', frontendState.auth);
    }

    // Merge (union) the backend table list with what we already have. Newer core
    // enumerates every local table (incl. internal `_00_*`); older core sends
    // only app tables. Merging means the panel's own `fetchTables()` result
    // (which always sees `_00_*`) isn't clobbered by a subsequent backend push.
    if (frontendState.database?.tables) {
      const incoming = frontendState.database.tables;
      setState('database', 'tables', (prev) => unionTables(incoming, prev));
    }

    // Update component versions
    if (frontendState.versions) {
      setState('versions', frontendState.versions);
    }
  }

  /**
   * Check if Sp00ky is available on the page
   */
  function checkSp00ky() {
    hostPage.checkSp00kyAvailable((available) => {
      console.log('[DevTools] Sp00ky available:', available);
      setIsSp00kyAvailable(available);

      if (available) {
        hostPage.getSp00kyState(
          (sp00kyState) => {
            if (sp00kyState) {
              updateState(sp00kyState);
            }
          },
          (error) => {
            console.error('[DevTools] Error getting Sp00ky state:', error);
          }
        );
      }
    });
  }

  /**
   * Clear all events - clears both local state and backend history
   */
  function clearEvents() {
    // Clear backend history first
    hostPage.clearHistory(
      (result) => {
        console.log('[DevTools] Clear history result:', result);
      },
      (error) => {
        console.error('[DevTools] Error clearing history:', error);
      }
    );
    // Clear local state immediately for responsive UI
    setState('events', []);
  }

  /**
   * Refresh state from the page
   */
  function refresh() {
    checkSp00ky();
    // The single top-right Refresh also re-runs backend version discovery so the
    // Versions tab no longer needs its own (removed) button.
    refreshVersions();
    // Note: the table list refreshes via the backend's getState() (checkSp00ky
    // above), so we don't run the panel's own (unreliable) schema queries here.
    const currentTable = selectedTable();
    if (currentTable) {
      fetchTableData(currentTable);
    }
  }

  /**
   * Re-run backend version discovery in the page. Core's refreshVersions()
   * re-fetches the ssp/scheduler/surrealdb versions and posts a state change,
   * which arrives back through the normal SP00KY_STATE_CHANGED channel.
   */
  function refreshVersions() {
    hostPage.run(
      `(async function() {
        if (window.__00__ && window.__00__.refreshVersions) {
          await window.__00__.refreshVersions();
          return { success: true };
        }
        return { success: false };
      })()`,
      {
        onError: (error) => console.error('[DevTools] Error refreshing versions:', error),
      }
    );
  }

  /**
   * Fetch table data from the page
   */
  function fetchTableData(tableName: string) {
    console.log('[DevTools] Fetching table data for:', tableName);
    hostPage.getTableData(
      tableName,
      (result) => {
        console.log('[DevTools] Table data fetch result:', result);
      },
      (error) => {
        console.error('[DevTools] Error fetching table data:', error);
      }
    );
  }

  /**
   * Update a table row
   */
  function updateTableRow(tableName: string, recordId: string, updates: Record<string, unknown>) {
    console.log('[DevTools] Updating row:', { tableName, recordId, updates });
    hostPage.updateTableRow(
      tableName,
      recordId,
      updates,
      (result) => {
        console.log('[DevTools] Update result:', result);
        if (result.success) {
          // Refresh table data after successful update
          fetchTableData(tableName);
        } else {
          console.error('[DevTools] Update failed:', result.error);
        }
      },
      (error) => {
        console.error('[DevTools] Error updating row:', error);
      }
    );
  }

  /**
   * Delete a table row
   */
  function deleteTableRow(tableName: string, recordId: string) {
    console.log('[DevTools] Deleting row:', { tableName, recordId });
    hostPage.deleteTableRow(
      tableName,
      recordId,
      (result) => {
        console.log('[DevTools] Delete result:', result);
        if (result.success) {
          // Refresh table data after successful delete
          fetchTableData(tableName);
        } else {
          console.error('[DevTools] Delete failed:', result.error);
        }
      },
      (error) => {
        console.error('[DevTools] Error deleting row:', error);
      }
    );
  }

  // Check for Sp00ky on mount
  onMount(() => {
    setTimeout(() => {
      checkSp00ky();
    }, 500);

    // Listen for window messages (table data responses)
    const handleWindowMessage = (event: MessageEvent) => {
      if (event.data.source === 'sp00ky-devtools-page') {
        handleMessage(event.data as ChromeMessage);
      }
    };

    window.addEventListener('message', handleWindowMessage);

    return () => {
      window.removeEventListener('message', handleWindowMessage);
    };
  });

  const runQuery = (query: string, target: 'local' | 'remote') => {
    return new Promise<{ success: boolean; data: any; error?: string }>((resolve, reject) => {
      const requestId = Math.random().toString(36).substring(7);

      // Timeout handling
      const timeoutId = setTimeout(() => {
        if (pendingQueries.has(requestId)) {
          pendingQueries.delete(requestId);
          console.error('[DevToolsContext] Query timed out:', requestId);
          reject('Query timed out (10s)');
        }
      }, 10000); // 10s timeout

      pendingQueries.set(requestId, {
        resolve: (data) => {
          clearTimeout(timeoutId);
          resolve(data);
        },
        reject: (err) => {
          clearTimeout(timeoutId);
          const safeErr = err || 'Undefined error passed to pendingQueries.reject';
          console.error('[DevToolsContext] Rejecting query', requestId, 'with:', safeErr);
          reject(safeErr);
        },
      });

      // Use eval to trigger the event directly in the page
      // This bypasses potential message dropping in background script
      console.log(
        '[DevToolsContext] Triggering RUN_QUERY via hostPage.runQuery (eval event dispatch)',
        requestId
      );

      hostPage.runQuery(
        query,
        target,
        requestId,
        (result) => {
          if (result && !result.success) {
            clearTimeout(timeoutId);
            pendingQueries.delete(requestId);
            const safeErr = result.error || 'Failed to dispatch query event';
            console.error('[DevToolsContext] Event dispatch failed:', safeErr);
            reject(safeErr);
          }
        },
        (err) => {
          clearTimeout(timeoutId);
          pendingQueries.delete(requestId);
          const errorStr = err instanceof Error ? err.message : String(err);
          console.error('[DevToolsContext] Eval error:', errorStr);
          reject(errorStr);
        }
      );
    });
  };

  /**
   * Cheap: fetch just the table list via a single `INFO FOR DB`. Safe to call
   * often (Refresh, Database tab open, "show internal" toggle) — guarded so
   * overlapping calls don't pile up on the query channel.
   */
  const fetchTables = async (target: 'local' | 'remote' = 'local') => {
    if (target === 'local' ? tablesInFlight : remoteTablesInFlight) return;
    if (target === 'local') tablesInFlight = true;
    else remoteTablesInFlight = true;

    // Remote enumeration can be denied (the session has no permission to run
    // `INFO FOR DB` remotely) or simply unreachable. When it fails, mirror the
    // local table list into `remoteTables` so the Remote picker still shows
    // something instead of an empty list.
    const fallbackRemoteToLocal = () => {
      if (target !== 'remote') return;
      console.warn(
        '[DevToolsContext] Remote table enumeration failed; falling back to local tables'
      );
      setState('database', 'remoteTables', state.database.tables);
    };

    try {
      const infoRes = await runQuery('INFO FOR DB', target);

      // Handle SurrealDB response format: [{ status: 'OK', result: { tables: ... } }] or [[{ tables: ... }]]
      if (!Array.isArray(infoRes) || !infoRes[0]) {
        console.warn('[DevToolsContext] INFO FOR DB failed or invalid format', infoRes);
        fallbackRemoteToLocal();
        return;
      }

      let info: any = null;
      if ('result' in infoRes[0]) {
        info = infoRes[0].result;
      } else if (Array.isArray(infoRes[0])) {
        info = infoRes[0][0]; // Unwrap nested array
      } else {
        info = infoRes[0]; // Fallback
      }

      if (!info || !info.tables) {
        console.warn('[DevToolsContext] No tables found in INFO FOR DB result', info);
        fallbackRemoteToLocal();
        return;
      }

      const tables = Object.keys(info.tables);
      if (target === 'remote') {
        // Remote isn't pushed by the backend, so this fetch is the source of
        // truth — replace (don't union) so nonexistent tables don't linger.
        setState('database', 'remoteTables', tables);
      } else {
        // Merge so a later backend push (which may omit `_00_*`) can't drop them.
        setState('database', 'tables', (prev) => unionTables(prev, tables));
      }
    } catch (e) {
      console.error(`[DevToolsContext] fetchTables(${target}) failed:`, e);
      fallbackRemoteToLocal();
    } finally {
      if (target === 'local') tablesInFlight = false;
      else remoteTablesInFlight = false;
    }
  };

  /**
   * Full schema: table list + per-table field lists (`INFO FOR TABLE`). This is
   * the heavy one (a query per table) — run once when Sp00ky becomes available,
   * NOT on every UI interaction. Guarded against overlapping runs.
   */
  const fetchSchema = async () => {
    if (schemaInFlight) return;
    schemaInFlight = true;
    try {
      console.log('[DevToolsContext] Fetching DB Schema...');
      await fetchTables();
      const tables = state.database.tables;

      const schema: Record<string, string[]> = {};

      // For each table, get columns via INFO FOR TABLE. Batched so we don't fire
      // dozens of concurrent queries at the single WASM connection.
      const BATCH = 4;
      for (let i = 0; i < tables.length; i += BATCH) {
        await Promise.all(
          tables.slice(i, i + BATCH).map(async (table) => {
            try {
              const tableRes = await runQuery(`INFO FOR TABLE ${table}`, 'local');

              if (Array.isArray(tableRes) && tableRes[0]) {
                // Normalize nested vs wrapped
                const tableInfo =
                  'result' in tableRes[0]
                    ? tableRes[0].result
                    : Array.isArray(tableRes[0])
                      ? tableRes[0][0]
                      : tableRes[0];

                if (tableInfo && tableInfo.fields) {
                  schema[table] = Object.keys(tableInfo.fields);
                } else {
                  schema[table] = []; // No explicit fields
                }
              }
            } catch (e) {
              console.error(`[DevToolsContext] Failed to fetch info for table ${table}`, e);
              schema[table] = [];
            }
          })
        );
      }

      console.log('[DevToolsContext] Schema fetched:', schema);
      setState('database', 'schema', schema);
    } catch (e) {
      console.error('[DevToolsContext] fetchSchema failed:', e);
    } finally {
      schemaInFlight = false;
    }
  };

  function setMcpEnabledAction(enabled: boolean) {
    sendMessage({ type: 'SET_MCP_ENABLED', enabled } as any);
  }

  const contextValue: DevToolsContextValue = {
    state,
    activeTab,
    selectedQueryHash,
    selectedTable,
    isSp00kyAvailable,
    mcpStatus,
    setMcpEnabled: setMcpEnabledAction,
    setActiveTab,
    setSelectedQueryHash,
    setSelectedTable,
    clearEvents,
    refresh,
    refreshVersions,
    fetchTableData,
    updateTableRow,
    deleteTableRow,
    runQuery: runQuery as any, // Cast to match interface if needed
    fetchSchema,
    fetchTables,
  };

  return <DevToolsContext.Provider value={contextValue}>{props.children}</DevToolsContext.Provider>;
};

export function useDevTools() {
  const context = useContext(DevToolsContext);
  if (!context) {
    throw new Error('useDevTools must be used within DevToolsProvider');
  }
  return context;
}
