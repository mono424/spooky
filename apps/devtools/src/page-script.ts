// This script runs in the page context and has access to window.__SPOOKY__
(function () {
  let isInitialized = false;

  // Hook into Spooky if it exists
  function checkForSpooky() {
    if ((window as any).__SPOOKY__ && !isInitialized) {
      console.log('Spooky detected by DevTools');
      isInitialized = true;

      const spooky = (window as any).__SPOOKY__;

      // Send initial detection message with full state
      window.postMessage(
        {
          type: 'SPOOKY_DETECTED',
          source: 'spooky-devtools-page',
          data: {
            version: spooky.version,
            detected: true,
            state: spooky.getState ? spooky.getState() : null,
          },
        },
        '*'
      );

      return true;
    }
    return false;
  }

  // Listen for state change messages from Spooky DevTools Service
  window.addEventListener('message', (event) => {
    // Only handle messages from the same window (from Spooky DevTools Service)
    if (event.source !== window) return;

    // Forward SPOOKY_STATE_CHANGED messages with full state to content script
    if (
      event.data.type === 'SPOOKY_STATE_CHANGED' &&
      event.data.source === 'spooky-devtools-page'
    ) {
      // The state is already included in the message from devtools-service
      // Just forward it as-is
    }

    // Forward SPOOKY_DETECTED messages
    if (event.data.type === 'SPOOKY_DETECTED' && event.data.source === 'spooky-devtools-page') {
      // Already being handled, no need to duplicate
    }
  });

  // Listen for GET_STATE requests
  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    if (event.data.type === 'GET_STATE' && event.data.source === 'spooky-devtools-content') {
      if ((window as any).__SPOOKY__) {
        const spooky = (window as any).__SPOOKY__;
        window.postMessage(
          {
            type: 'SPOOKY_STATE_CHANGED',
            source: 'spooky-devtools-page',
            state: spooky.getState ? spooky.getState() : null,
          },
          '*'
        );
      }
    }
  });

  // Listen for execution requests from content script
  window.addEventListener('SPOOKY_RUN_QUERY', async (event: any) => {
    const { requestId, query, target } = event.detail;

    if (!(window as any).__SPOOKY__?.runQuery) {
      window.postMessage(
        {
          type: 'SPOOKY_QUERY_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: 'Spooky not found or runQuery not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await (window as any).__SPOOKY__.runQuery(query, target);
      window.postMessage(
        {
          type: 'SPOOKY_QUERY_RESPONSE',
          source: 'spooky-devtools-page',
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
          type: 'SPOOKY_QUERY_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Try immediately
  if (!checkForSpooky()) {
    // If not found, try again after a short delay
    setTimeout(checkForSpooky, 100);
    setTimeout(checkForSpooky, 500);
    setTimeout(checkForSpooky, 1000);
    setTimeout(checkForSpooky, 2000);
  }

  // Listen for GET_TABLE_DATA requests from content script
  window.addEventListener('SPOOKY_GET_TABLE_DATA', async (event: any) => {
    const { requestId, tableName } = event.detail;
    const spooky = (window as any).__SPOOKY__;

    if (!spooky?.getTableData) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: 'Spooky not found or getTableData not supported',
        },
        '*'
      );
      return;
    }

    try {
      const data = await spooky.getTableData(tableName);
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: true,
          data,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for UPDATE_TABLE_ROW requests from content script
  window.addEventListener('SPOOKY_UPDATE_TABLE_ROW', async (event: any) => {
    const { requestId, tableName, recordId, updates } = event.detail;
    const spooky = (window as any).__SPOOKY__;

    if (!spooky?.updateTableRow) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: 'Spooky not found or updateTableRow not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await spooky.updateTableRow(tableName, recordId, updates);
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
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
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for DELETE_TABLE_ROW requests from content script
  window.addEventListener('SPOOKY_DELETE_TABLE_ROW', async (event: any) => {
    const { requestId, tableName, recordId } = event.detail;
    const spooky = (window as any).__SPOOKY__;

    if (!spooky?.deleteTableRow) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: 'Spooky not found or deleteTableRow not supported',
        },
        '*'
      );
      return;
    }

    try {
      const result = await spooky.deleteTableRow(tableName, recordId);
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
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
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Listen for CLEAR_HISTORY requests from content script
  window.addEventListener('SPOOKY_CLEAR_HISTORY', (event: any) => {
    const { requestId } = event.detail;
    const spooky = (window as any).__SPOOKY__;

    if (!spooky?.clearHistory) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: 'Spooky not found or clearHistory not supported',
        },
        '*'
      );
      return;
    }

    try {
      spooky.clearHistory();
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: true,
        },
        '*'
      );
    } catch (err: any) {
      window.postMessage(
        {
          type: 'SPOOKY_BRIDGE_RESPONSE',
          source: 'spooky-devtools-page',
          requestId,
          success: false,
          error: err.message || String(err),
        },
        '*'
      );
    }
  });

  // Also listen for custom event in case Spooky loads later
  window.addEventListener('spooky:init', () => {
    checkForSpooky();
  });
})();
