import { createSignal } from 'solid-js';

export interface RunInHostPageOptions<T> {
  onSuccess?: (result: T) => void;
  onError?: (error: any) => void;
}

/**
 * Custom hook to run code in the host page context using chrome.devtools.inspectedWindow.eval
 * This is a safer alternative to directly calling eval and handles the callback pattern reactively
 */
export function useRunInHostPage() {
  const [isRunning, setIsRunning] = createSignal(false);
  const [error, setError] = createSignal<any>(null);

  /**
   * Execute code in the inspected page
   * @param code - JavaScript code to execute in the host page
   * @param options - Success and error callbacks
   */
  const run = <T = any>(code: string, options?: RunInHostPageOptions<T>): void => {
    setIsRunning(true);
    setError(null);

    chrome.devtools.inspectedWindow.eval(code, (result: T, isException: any) => {
      setIsRunning(false);

      if (isException) {
        setError(isException);
        options?.onError?.(isException);
      } else {
        options?.onSuccess?.(result);
      }
    });
  };

  /**
   * Get the Spooky state from the host page
   */
  const getSpookyState = (
    onSuccess: (state: any) => void,
    onError?: (error: any) => void
  ): void => {
    run(`window.__SPOOKY__ ? window.__SPOOKY__.getState() : null`, {
      onSuccess,
      onError,
    });
  };

  /**
   * Get table data from the host page
   */
  const getTableData = (
    tableName: string,
    onSuccess: (data: Record<string, unknown>[]) => void,
    onError?: (error: any) => void
  ): void => {
    run(
      `(async function() {
        try {
          if (window.__SPOOKY__ && window.__SPOOKY__.getTableData) {
            const data = await window.__SPOOKY__.getTableData("${tableName}");
            window.postMessage({
              type: 'SPOOKY_TABLE_DATA_RESPONSE',
              source: 'spooky-devtools-page',
              tableName: "${tableName}",
              data: data
            }, '*');
            return { success: true, count: data?.length || 0 };
          }
          return { success: false, error: 'Spooky not found' };
        } catch (error) {
          return { success: false, error: error instanceof Error ? error.message : String(error) };
        }
      })()`,
      { onSuccess, onError }
    );
  };

  /**
   * Clear events history in the host page
   */
  const clearHistory = (
    onSuccess?: (result: { success: boolean }) => void,
    onError?: (error: any) => void
  ): void => {
    run(
      `(function() {
        if (window.__SPOOKY__ && window.__SPOOKY__.clearHistory) {
          window.__SPOOKY__.clearHistory();
          return { success: true };
        }
        return { success: false };
      })()`,
      { onSuccess, onError }
    );
  };

  /**
   * Check if Spooky is available on the page
   */
  const checkSpookyAvailable = (onSuccess: (available: boolean) => void): void => {
    run(`!!window.__SPOOKY__`, { onSuccess });
  };

  /**
   * Update a table row
   */
  const updateTableRow = (
    tableName: string,
    recordId: string,
    updates: Record<string, unknown>,
    onSuccess: (result: { success: boolean; error?: string }) => void,
    onError?: (error: any) => void
  ): void => {
    const updatesJson = JSON.stringify(updates).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    run(
      `(async function() {
        try {
          if (window.__SPOOKY__ && window.__SPOOKY__.updateTableRow) {
            const updates = JSON.parse("${updatesJson}");
            const result = await window.__SPOOKY__.updateTableRow("${tableName}", "${recordId}", updates);
            return result;
          }
          return { success: false, error: 'Spooky not found' };
        } catch (error) {
          return { success: false, error: error instanceof Error ? error.message : String(error) };
        }
      })()`,
      { onSuccess, onError }
    );
  };

  /**
   * Delete a table row
   */
  const deleteTableRow = (
    tableName: string,
    recordId: string,
    onSuccess: (result: { success: boolean; error?: string }) => void,
    onError?: (error: any) => void
  ): void => {
    run(
      `(async function() {
        try {
          if (window.__SPOOKY__ && window.__SPOOKY__.deleteTableRow) {
            const result = await window.__SPOOKY__.deleteTableRow("${tableName}", "${recordId}");
            return result;
          }
          return { success: false, error: 'Spooky not found' };
        } catch (error) {
          return { success: false, error: error instanceof Error ? error.message : String(error) };
        }
      })()`,
      { onSuccess, onError }
    );
  };

  /**
   * Run a query in the host page
   */
  const runQuery = (
    query: string,
    target: 'local' | 'remote',
    requestId: string,
    onSuccess: (result: { success: boolean; data?: any; error?: string }) => void,
    onError?: (error: any) => void
  ): void => {
    // Escape query for eval
    const escapedQuery = query.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, ' ');

    // We strictly use eval to DISPATCH THE EVENT.
    // The actual execution happens in page-script.ts (which listens to this event).
    // This avoids all async/await serialization issues in eval.
    run(
      `(function() {
        try {
            window.dispatchEvent(new CustomEvent('SPOOKY_RUN_QUERY', {
                detail: {
                    requestId: '${requestId}',
                    query: "${escapedQuery}",
                    target: "${target}"
                }
            }));
            return { success: true, started: true };
        } catch (error) {
            var msg = error instanceof Error ? error.message : String(error);
            return { success: false, error: msg || 'Unknown caught error in eval dispatch' };
        }
      })()`,
      { onSuccess, onError }
    );
  };

  return {
    run,
    getSpookyState,
    getTableData,
    runQuery,
    updateTableRow,
    deleteTableRow,
    clearHistory,
    checkSpookyAvailable,
    isRunning,
    error,
  };
}
