// This script runs in the page context and has access to window.__SP00KY__
(function () {
  let isInitialized = false;

  // Hook into Sp00ky if it exists
  function checkForSp00ky() {
    if ((window as any).__SP00KY__ && !isInitialized) {
      console.log('Sp00ky detected by DevTools');
      isInitialized = true;

      const sp00ky = (window as any).__SP00KY__;

      // Send initial detection message with full state
      window.postMessage(
        {
          type: 'SP00KY_DETECTED',
          source: 'sp00ky-devtools-page',
          data: {
            version: sp00ky.version,
            detected: true,
            state: sp00ky.getState ? sp00ky.getState() : null,
          },
        },
        '*'
      );

      return true;
    }
    return false;
  }

  // Listen for state change messages from Sp00ky DevTools Service
  window.addEventListener('message', (event) => {
    // Only handle messages from the same window (from Sp00ky DevTools Service)
    if (event.source !== window) return;

    // Forward SP00KY_STATE_CHANGED messages with full state to content script
    if (
      event.data.type === 'SP00KY_STATE_CHANGED' &&
      event.data.source === 'sp00ky-devtools-page'
    ) {
      // The state is already included in the message from devtools-service
      // Just forward it as-is
    }

    // Forward SP00KY_DETECTED messages
    if (event.data.type === 'SP00KY_DETECTED' && event.data.source === 'sp00ky-devtools-page') {
      // Already being handled, no need to duplicate
    }
  });

  // Listen for GET_STATE requests
  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    if (event.data.type === 'GET_STATE' && event.data.source === 'sp00ky-devtools-content') {
      if ((window as any).__SP00KY__) {
        const sp00ky = (window as any).__SP00KY__;
        window.postMessage(
          {
            type: 'SP00KY_STATE_CHANGED',
            source: 'sp00ky-devtools-page',
            state: sp00ky.getState ? sp00ky.getState() : null,
          },
          '*'
        );
      }
    }
  });

  // Listen for execution requests from content script
  window.addEventListener('SP00KY_RUN_QUERY', async (event: any) => {
    const { requestId, query, target } = event.detail;

    if (!(window as any).__SP00KY__?.runQuery) {
      window.postMessage(
        {
          type: 'SP00KY_QUERY_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: 'Sp00ky not found or runQuery not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await (window as any).__SP00KY__.runQuery(query, target);
      window.postMessage(
        {
          type: 'SP00KY_QUERY_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: result.success,
          data: result.data,
          error: result.error,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SP00KY_QUERY_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Try immediately
  if (!checkForSp00ky()) {
    // If not found, try again after a short delay
    setTimeout(checkForSp00ky, 100);
    setTimeout(checkForSp00ky, 500);
    setTimeout(checkForSp00ky, 1000);
    setTimeout(checkForSp00ky, 2000);
  }

  // Listen for GET_TABLE_DATA requests from content script
  window.addEventListener('SP00KY_GET_TABLE_DATA', async (event: any) => {
    const { requestId, tableName } = event.detail;
    const sp00ky = (window as any).__SP00KY__;

    if (!sp00ky?.getTableData) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: 'Sp00ky not found or getTableData not supported',
        },
        '*'
      );
      return;
    }

    try {
      const data = await sp00ky.getTableData(tableName);
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: true,
          data,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for UPDATE_TABLE_ROW requests from content script
  window.addEventListener('SP00KY_UPDATE_TABLE_ROW', async (event: any) => {
    const { requestId, tableName, recordId, updates } = event.detail;
    const sp00ky = (window as any).__SP00KY__;

    if (!sp00ky?.updateTableRow) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: 'Sp00ky not found or updateTableRow not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await sp00ky.updateTableRow(tableName, recordId, updates);
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: result.success !== false,
          data: result,
          error: result.error,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for DELETE_TABLE_ROW requests from content script
  window.addEventListener('SP00KY_DELETE_TABLE_ROW', async (event: any) => {
    const { requestId, tableName, recordId } = event.detail;
    const sp00ky = (window as any).__SP00KY__;

    if (!sp00ky?.deleteTableRow) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: 'Sp00ky not found or deleteTableRow not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await sp00ky.deleteTableRow(tableName, recordId);
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: result.success !== false,
          data: result,
          error: result.error,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for CLEAR_HISTORY requests from content script
  window.addEventListener('SP00KY_CLEAR_HISTORY', (event: any) => {
    const { requestId } = event.detail;
    const sp00ky = (window as any).__SP00KY__;

    if (!sp00ky?.clearHistory) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: 'Sp00ky not found or clearHistory not supported',
        },
        '*'
      );
      return;
    }

    try {
      sp00ky.clearHistory();
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: true,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SP00KY_BRIDGE_RESPONSE',
          source: 'sp00ky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Also listen for custom event in case Sp00ky loads later
  window.addEventListener('sp00ky:init', () => {
    checkForSp00ky();
  });
})();
