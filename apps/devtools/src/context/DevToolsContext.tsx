import {
  createContext,
  useContext,
  createSignal,
  onMount,
  createEffect,
  type ParentComponent,
} from "solid-js";
import { createStore } from "solid-js/store";
import type {
  DevToolsState,
  BackendDevToolsState,
  TabType,
  ChromeMessage,
} from "../types/devtools";
import { useChromeConnection } from "../hooks/useChromeConnection";
import { useRunInHostPage } from "../hooks/useRunInHostPage";
import { adaptBackendState } from "../utils/state-adapter";

interface DevToolsContextValue {
  // State
  state: DevToolsState;
  activeTab: () => TabType;
  selectedQueryHash: () => number | null;
  selectedTable: () => string | null;
  isSpookyAvailable: () => boolean;

  // Actions
  setActiveTab: (tab: TabType) => void;
  setSelectedQueryHash: (hash: number | null) => void;
  setSelectedTable: (table: string | null) => void;
  clearEvents: () => void;
  refresh: () => void;
  fetchTableData: (tableName: string) => void;
  updateTableRow: (
    tableName: string,
    recordId: string,
    updates: Record<string, unknown>
  ) => void;
  deleteTableRow: (tableName: string, recordId: string) => void;
  runQuery?: (query: string, target: 'local' | 'remote') => Promise<any>;
  fetchSchema?: () => Promise<void>;
}

const DevToolsContext = createContext<DevToolsContextValue>();

export const DevToolsProvider: ParentComponent = (props) => {
  // Store for DevTools state
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
      tableData: {},
    },
  });

  // UI state
  const [activeTab, setActiveTab] = createSignal<TabType>("events");
  const [selectedQueryHash, setSelectedQueryHash] = createSignal<number | null>(
    null
  );
  const [selectedTable, setSelectedTable] = createSignal<string | null>(null);
  const [isSpookyAvailable, setIsSpookyAvailable] = createSignal(false);

  // Custom hooks
  const { requestState } = useChromeConnection({
    onMessage: handleMessage,
    onConnect: () => {
      console.log("[DevTools] Chrome connection established");
      checkSpooky();
    },
    onDisconnect: () => {
      console.log("[DevTools] Chrome connection lost");
      setIsSpookyAvailable(false);
    },
  });

  const hostPage = useRunInHostPage();

  /**
   * Handle messages from background script
   */
  // Stores pending query requests: requestId -> { resolve, reject }
  const pendingQueries = new Map<string, { resolve: (data: any) => void; reject: (err: string) => void }>();

  /**
   * Handle messages from background script
   */
  function handleMessage(message: ChromeMessage) {
    console.log("[DevTools] Processing message:", message);

    switch (message.type) {
      case "SPOOKY_DETECTED":
        setIsSpookyAvailable(true);
        // If state is included in the detection message, use it
        if (message.data && (message.data as any).state) {
            console.log("[DevToolsContext] Initialized with state from detection");
            updateState((message.data as any).state);
        } else {
            console.log("[DevToolsContext] Spooky detected, requesting state...");
            requestState();
        }
        break;

      case "SPOOKY_STATE_CHANGED":
        if (message.state) {
          console.log("[DevToolsContext] State updated. Tables:", message.state.database?.tables?.length || 0);
          updateState(message.state);
        }
        break;

      case "SPOOKY_TABLE_DATA_RESPONSE":
        if (message.tableName && message.data) {
          setState("database", "tableData", message.tableName, message.data as Record<string, unknown>[]);
        }
        break;

      case "SPOOKY_QUERY_RESPONSE":
         // @ts-ignore - Validating custom message structure
        const msg = message as any;
        console.log("[DevTools] RAW QUERY RESPONSE:", msg);

        if (msg.requestId && pendingQueries.has(msg.requestId)) {
            const { resolve, reject } = pendingQueries.get(msg.requestId)!;
            pendingQueries.delete(msg.requestId);
            
            if (msg.success) {
                resolve(msg.data);
            } else {
                console.error("[DevTools] Rejecting with error:", msg.error);
                reject(msg.error || "Unknown error from query response (msg.error was falsy)");
            }
        } else {
            console.warn("[DevTools] Received query response for unknown/expired requestId:", msg.requestId);
        }
        break;

      case "PAGE_RELOADED":
        console.log("[DevTools] Page reloaded, checking for Spooky...");
        setTimeout(() => {
          checkSpooky();
        }, 500);
        // Clear pending queries on reload
        pendingQueries.forEach(({ reject }) => reject("Page reloaded"));
        pendingQueries.clear();
        break;

      default:
        console.log("[DevTools] Unknown message type:", message.type);
    }
  }

  /**
   * Update state from Spooky - accepts backend state format
   */
  function updateState(backendState: BackendDevToolsState | DevToolsState) {
    console.log("[DevTools] Received state:", backendState);

    // Check if it's backend format (has eventsHistory) or frontend format (has events)
    const frontendState =
      "eventsHistory" in backendState
        ? adaptBackendState(backendState as BackendDevToolsState)
        : (backendState as DevToolsState);

    console.log("[DevTools] Adapted state:", frontendState);

    // Update events
    if (frontendState.events) {
      setState("events", frontendState.events);
    }

    // Update active queries
    if (frontendState.activeQueries) {
      setState("activeQueries", frontendState.activeQueries);
    }

    // Update auth
    if (frontendState.auth) {
      setState("auth", frontendState.auth);
    }

    // Update database tables list
    if (frontendState.database?.tables) {
      setState("database", "tables", frontendState.database.tables);
    }
  }

  /**
   * Check if Spooky is available on the page
   */
  function checkSpooky() {
    hostPage.checkSpookyAvailable((available) => {
      console.log("[DevTools] Spooky available:", available);
      setIsSpookyAvailable(available);

      if (available) {
        hostPage.getSpookyState(
          (state) => {
            if (state) {
              updateState(state);
            }
          },
          (error) => {
            console.error("[DevTools] Error getting Spooky state:", error);
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
        console.log("[DevTools] Clear history result:", result);
      },
      (error) => {
        console.error("[DevTools] Error clearing history:", error);
      }
    );
    // Clear local state immediately for responsive UI
    setState("events", []);
  }

  /**
   * Refresh state from the page
   */
  function refresh() {
    checkSpooky();
    const currentTable = selectedTable();
    if (currentTable) {
      fetchTableData(currentTable);
    }
  }

  /**
   * Fetch table data from the page
   */
  function fetchTableData(tableName: string) {
    console.log("[DevTools] Fetching table data for:", tableName);
    hostPage.getTableData(
      tableName,
      (result) => {
        console.log("[DevTools] Table data fetch result:", result);
      },
      (error) => {
        console.error("[DevTools] Error fetching table data:", error);
      }
    );
  }

  /**
   * Update a table row
   */
  function updateTableRow(
    tableName: string,
    recordId: string,
    updates: Record<string, unknown>
  ) {
    console.log("[DevTools] Updating row:", { tableName, recordId, updates });
    hostPage.updateTableRow(
      tableName,
      recordId,
      updates,
      (result) => {
        console.log("[DevTools] Update result:", result);
        if (result.success) {
          // Refresh table data after successful update
          fetchTableData(tableName);
        } else {
          console.error("[DevTools] Update failed:", result.error);
        }
      },
      (error) => {
        console.error("[DevTools] Error updating row:", error);
      }
    );
  }

  /**
   * Delete a table row
   */
  function deleteTableRow(tableName: string, recordId: string) {
    console.log("[DevTools] Deleting row:", { tableName, recordId });
    hostPage.deleteTableRow(
      tableName,
      recordId,
      (result) => {
        console.log("[DevTools] Delete result:", result);
        if (result.success) {
          // Refresh table data after successful delete
          fetchTableData(tableName);
        } else {
          console.error("[DevTools] Delete failed:", result.error);
        }
      },
      (error) => {
        console.error("[DevTools] Error deleting row:", error);
      }
    );
  }

  // Check for Spooky on mount
  onMount(() => {
    setTimeout(() => {
      checkSpooky();
    }, 500);

    // Listen for window messages (table data responses)
    const handleWindowMessage = (event: MessageEvent) => {
      if (event.data.source === "spooky-devtools-page") {
        handleMessage(event.data as ChromeMessage);
      }
    };

    window.addEventListener("message", handleWindowMessage);

    return () => {
      window.removeEventListener("message", handleWindowMessage);
    };
    return () => {
      window.removeEventListener("message", handleWindowMessage);
    };
  });

  // Fetch schema when Spooky becomes available
  createEffect(() => {
      if (isSpookyAvailable()) {
          // Delay slightly to ensure everything is settled
          setTimeout(() => {
              fetchSchema();
          }, 500);
      }
  });

  const runQuery = (query: string, target: 'local' | 'remote') => {
      return new Promise<{success: boolean, data: any, error?: string}>((resolve, reject) => {
        const requestId = Math.random().toString(36).substring(7);
        
        // Timeout handling
        const timeoutId = setTimeout(() => {
            if (pendingQueries.has(requestId)) {
                pendingQueries.delete(requestId);
                console.error("[DevToolsContext] Query timed out:", requestId);
                reject("Query timed out (10s)");
            }
        }, 10000); // 10s timeout

        pendingQueries.set(requestId, {
            resolve: (data) => {
                clearTimeout(timeoutId);
                resolve(data);
            },
            reject: (err) => {
                clearTimeout(timeoutId);
                const safeErr = err || "Undefined error passed to pendingQueries.reject";
                console.error("[DevToolsContext] Rejecting query", requestId, "with:", safeErr);
                reject(safeErr);
            }
        });

        // Use eval to trigger the event directly in the page
        // This bypasses potential message dropping in background script
        console.log("[DevToolsContext] Triggering RUN_QUERY via hostPage.runQuery (eval event dispatch)", requestId);
        
        hostPage.runQuery(
            query, 
            target, 
            requestId,
            (result) => {
                 if (result && !result.success) {
                    clearTimeout(timeoutId);
                    pendingQueries.delete(requestId);
                    const safeErr = result.error || "Failed to dispatch query event";
                    console.error("[DevToolsContext] Event dispatch failed:", safeErr);
                    reject(safeErr);
                 }
            },
            (err) => {
                clearTimeout(timeoutId);
                pendingQueries.delete(requestId);
                const errorStr = err instanceof Error ? err.message : String(err);
                console.error("[DevToolsContext] Eval error:", errorStr);
                reject(errorStr);
            }
        );
      });
  };

    const fetchSchema = async () => {
        try {
            console.log("[DevToolsContext] Fetching DB Schema...");
            // 1. Get Tables via INFO FOR DB
            const infoRes = await runQuery("INFO FOR DB", "local");
            
            // Handle SurrealDB response format: [{ status: 'OK', result: { tables: ... } }] or [[{ tables: ... }]]
            if (!Array.isArray(infoRes) || !infoRes[0]) {
                 console.warn("[DevToolsContext] INFO FOR DB failed or invalid format", infoRes);
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
                console.warn("[DevToolsContext] No tables found in INFO FOR DB result", info);
                return;
            }

            const tables = Object.keys(info.tables);
            // Update tables list immediately
             setState("database", "tables", tables);

            const schema: Record<string, string[]> = {};

            // 2. For each table, get columns via INFO FOR TABLE
            // Run in parallel
            await Promise.all(tables.map(async (table) => {
                try {
                    const tableRes = await runQuery(`INFO FOR TABLE ${table}`, "local");
                    
                    if (Array.isArray(tableRes) && tableRes[0]) {
                        // Normalize nested vs wrapped
                        const tableInfo = ('result' in tableRes[0]) 
                            ? tableRes[0].result 
                            : (Array.isArray(tableRes[0]) ? tableRes[0][0] : tableRes[0]);

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
            }));
            
            console.log("[DevToolsContext] Schema fetched:", schema);
            setState("database", "schema", schema);

        } catch (e) {
            console.error("[DevToolsContext] fetchSchema failed:", e);
        }
  };

  const contextValue: DevToolsContextValue = {
    state,
    activeTab,
    selectedQueryHash,
    selectedTable,
    isSpookyAvailable,
    setActiveTab,
    setSelectedQueryHash,
    setSelectedTable,
    clearEvents,
    refresh,
    fetchTableData,
    updateTableRow,
    deleteTableRow,
    runQuery: runQuery as any, // Cast to match interface if needed
    fetchSchema
  };

  return (
    <DevToolsContext.Provider value={contextValue}>
      {props.children}
    </DevToolsContext.Provider>
  );
};

export function useDevTools() {
  const context = useContext(DevToolsContext);
  if (!context) {
    throw new Error("useDevTools must be used within DevToolsProvider");
  }
  return context;
}
