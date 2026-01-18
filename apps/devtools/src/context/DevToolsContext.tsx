import {
  createContext,
  useContext,
  createSignal,
  onMount,
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
  function handleMessage(message: ChromeMessage) {
    console.log("[DevTools] Processing message:", message);

    switch (message.type) {
      case "SPOOKY_DETECTED":
        setIsSpookyAvailable(true);
        requestState();
        break;

      case "SPOOKY_STATE_CHANGED":
        if (message.state) {
          updateState(message.state);
        }
        break;

      case "SPOOKY_TABLE_DATA_RESPONSE":
        if (message.tableName && message.data) {
          setState("database", "tableData", message.tableName, message.data as Record<string, unknown>[]);
        }
        break;

      case "PAGE_RELOADED":
        console.log("[DevTools] Page reloaded, checking for Spooky...");
        setTimeout(() => {
          checkSpooky();
        }, 500);
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
  });

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
