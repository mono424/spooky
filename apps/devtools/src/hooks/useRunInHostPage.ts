import { createSignal } from "solid-js";

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
  const run = <T = any>(
    code: string,
    options?: RunInHostPageOptions<T>
  ): void => {
    setIsRunning(true);
    setError(null);

    chrome.devtools.inspectedWindow.eval(
      code,
      (result: T, isException: any) => {
        setIsRunning(false);

        if (isException) {
          setError(isException);
          options?.onError?.(isException);
        } else {
          options?.onSuccess?.(result);
        }
      }
    );
  };

  /**
   * Get the Spooky state from the host page
   */
  const getSpookyState = (
    onSuccess: (state: any) => void,
    onError?: (error: any) => void
  ): void => {
    run(
      `window.__SPOOKY__ ? window.__SPOOKY__.getState() : null`,
      {
        onSuccess,
        onError,
      }
    );
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
          return { success: false, error: error.message };
        }
      })()`,
      { onSuccess, onError }
    );
  };

  /**
   * Check if Spooky is available on the page
   */
  const checkSpookyAvailable = (
    onSuccess: (available: boolean) => void
  ): void => {
    run(
      `!!window.__SPOOKY__`,
      { onSuccess }
    );
  };

  return {
    run,
    getSpookyState,
    getTableData,
    checkSpookyAvailable,
    isRunning,
    error,
  };
}
