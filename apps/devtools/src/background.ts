// Background service worker for the extension
// Handles communication between content scripts and devtools panels

console.log('Sp00ky DevTools background script loaded');

// Keep track of active connections
const connections = new Map<number, chrome.runtime.Port>();

// --- MCP Bridge WebSocket Client ---

const BRIDGE_PORT = 9315;
let bridgeSocket: WebSocket | null = null;
let bridgeReconnectTimer: ReturnType<typeof setTimeout> | null = null;
let bridgeReconnectDelay = 1000;
const BRIDGE_MAX_RECONNECT_DELAY = 30000;
let mcpEnabled = false;

// Track tabs that have Sp00ky detected
const sp00kyTabs = new Map<number, { url?: string; title?: string }>();

function connectToBridge() {
  if (!mcpEnabled) return;
  if (bridgeSocket && bridgeSocket.readyState === WebSocket.OPEN) return;

  try {
    bridgeSocket = new WebSocket(`ws://127.0.0.1:${BRIDGE_PORT}`);
  } catch (err) {
    console.warn('[DevTools Bridge] Failed to create WebSocket:', err);
    scheduleBridgeReconnect();
    return;
  }

  bridgeSocket.onopen = () => {
    console.log('[DevTools Bridge] Connected to MCP bridge');
    bridgeReconnectDelay = 1000; // Reset backoff

    // Report connected tabs
    reportTabsToBridge();
    broadcastMcpStatus();
  };

  bridgeSocket.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data as string);
      handleBridgeRequest(msg);
    } catch (err) {
      console.warn('[DevTools Bridge] Bad message:', err);
    }
  };

  bridgeSocket.onclose = () => {
    console.log('[DevTools Bridge] Disconnected from MCP bridge');
    bridgeSocket = null;
    broadcastMcpStatus();
    scheduleBridgeReconnect();
  };

  bridgeSocket.onerror = (err) => {
    console.warn('[DevTools Bridge] WebSocket error:', err);
    // onclose will fire after this, triggering reconnect
  };
}

function scheduleBridgeReconnect() {
  if (!mcpEnabled) return;
  if (bridgeReconnectTimer) return;
  bridgeReconnectTimer = setTimeout(() => {
    bridgeReconnectTimer = null;
    bridgeReconnectDelay = Math.min(bridgeReconnectDelay * 1.5, BRIDGE_MAX_RECONNECT_DELAY);
    connectToBridge();
  }, bridgeReconnectDelay);
}

function disconnectFromBridge() {
  if (bridgeReconnectTimer) {
    clearTimeout(bridgeReconnectTimer);
    bridgeReconnectTimer = null;
  }
  if (bridgeSocket) {
    bridgeSocket.close();
    bridgeSocket = null;
  }
  bridgeReconnectDelay = 1000;
  broadcastMcpStatus();
}

function setMcpEnabled(enabled: boolean) {
  mcpEnabled = enabled;
  chrome.storage.local.set({ mcpEnabled: enabled });
  if (enabled) {
    connectToBridge();
  } else {
    disconnectFromBridge();
  }
}

function reportTabsToBridge() {
  if (!bridgeSocket || bridgeSocket.readyState !== WebSocket.OPEN) return;
  const tabs = Array.from(sp00kyTabs.entries()).map(([tabId, info]) => ({
    tabId,
    ...info,
  }));
  bridgeSocket.send(
    JSON.stringify({
      jsonrpc: '2.0',
      method: 'tabsChanged',
      params: { tabs },
    })
  );
}

// Map of pending bridge request IDs to their tab IDs
const pendingBridgeRequests = new Map<string, number>();

async function handleBridgeRequest(msg: any) {
  if (msg.jsonrpc !== '2.0' || !msg.id || !msg.method) return;

  const { id, method, params = {}, tabId: requestedTabId } = msg;

  // Resolve target tab
  let targetTabId: number | undefined = requestedTabId;
  if (targetTabId === undefined) {
    // Use first known sp00ky tab
    const firstTab = sp00kyTabs.keys().next().value;
    targetTabId = firstTab;
  }

  if (targetTabId === undefined) {
    sendBridgeError(id, -32000, 'No Sp00ky tabs connected');
    return;
  }

  // Handle getState by requesting state from the page
  if (method === 'getState') {
    // Register pending request, then ask content script for state
    pendingBridgeRequests.set(id, targetTabId);
    try {
      await chrome.tabs.sendMessage(targetTabId, {
        type: 'GET_SP00KY_STATE',
      });
    } catch (err: any) {
      pendingBridgeRequests.delete(id);
      sendBridgeError(id, -32000, `Failed to contact tab: ${err.message}`);
    }
    return;
  }

  // For methods that go through content script events and wait for a response
  const methodToType: Record<string, string> = {
    runQuery: 'RUN_QUERY',
    getTableData: 'GET_TABLE_DATA',
    updateTableRow: 'UPDATE_TABLE_ROW',
    deleteTableRow: 'DELETE_TABLE_ROW',
    clearHistory: 'CLEAR_HISTORY',
  };

  const msgType = methodToType[method];
  if (!msgType) {
    sendBridgeError(id, -32601, `Unknown method: ${method}`);
    return;
  }

  // Generate a requestId and track it
  const requestId = `bridge-${id}`;
  pendingBridgeRequests.set(requestId, targetTabId);

  try {
    await chrome.tabs.sendMessage(targetTabId, {
      type: msgType,
      payload: { ...params, requestId },
    });
  } catch (err: any) {
    pendingBridgeRequests.delete(requestId);
    sendBridgeError(id, -32000, `Failed to contact tab: ${err.message}`);
  }
}

function sendBridgeResponse(id: string, result: unknown) {
  if (!bridgeSocket || bridgeSocket.readyState !== WebSocket.OPEN) return;
  bridgeSocket.send(JSON.stringify({ jsonrpc: '2.0', id, result }));
}

function sendBridgeError(id: string, code: number, message: string) {
  if (!bridgeSocket || bridgeSocket.readyState !== WebSocket.OPEN) return;
  bridgeSocket.send(JSON.stringify({ jsonrpc: '2.0', id, error: { code, message } }));
}

// Keepalive ping to prevent service worker termination
setInterval(() => {
  if (bridgeSocket && bridgeSocket.readyState === WebSocket.OPEN) {
    // Send a lightweight ping message (WebSocket API handles pong automatically)
    bridgeSocket.send(JSON.stringify({ jsonrpc: '2.0', method: 'ping', params: {} }));
  }
}, 20_000);

// Load MCP enabled state and connect if enabled
chrome.storage.local.get('mcpEnabled', (result) => {
  mcpEnabled = result.mcpEnabled === true;
  if (mcpEnabled) {
    connectToBridge();
  }
});

function getMcpStatus() {
  return {
    type: 'MCP_STATUS',
    enabled: mcpEnabled,
    connected: bridgeSocket !== null && bridgeSocket.readyState === WebSocket.OPEN,
    port: BRIDGE_PORT,
  };
}

function broadcastMcpStatus() {
  const status = getMcpStatus();
  for (const port of connections.values()) {
    port.postMessage(status);
  }
}

// --- End MCP Bridge ---

// Handle connections from devtools panels
chrome.runtime.onConnect.addListener((port) => {
  console.log('DevTools panel connected');

  let tabId: number | undefined;

  // Listen for messages from the devtools panel
  const messageListener = (message: any) => {
    if (message.name === 'init') {
      tabId = message.tabId;
      if (tabId !== undefined) {
        connections.set(tabId, port);
      }
    }

    // Handle MCP status request from panel
    if (message.type === 'GET_MCP_STATUS') {
      port.postMessage(getMcpStatus());
      return;
    }

    // Handle MCP enable/disable from panel
    if (message.type === 'SET_MCP_ENABLED') {
      setMcpEnabled(!!message.enabled);
      // Broadcast after a short delay to let connect/disconnect settle
      setTimeout(() => broadcastMcpStatus(), 100);
      return;
    }

    // Forward messages to the content script
    if (tabId) {
      if (message.type === 'RUN_QUERY') {
        console.log('[DevTools Background] Forwarding RUN_QUERY to tab', tabId);
      }
      chrome.tabs.sendMessage(tabId, message).catch((error) => {
        // Ignore errors if content script is not ready or tab is closed
        console.warn('Failed to send message to content script:', error);
      });
    } else {
      console.warn('[DevTools Background] Dropping message, no tabId for port', message);
    }
  };

  port.onMessage.addListener(messageListener);

  port.onDisconnect.addListener(() => {
    console.log('DevTools panel disconnected');
    if (tabId) {
      connections.delete(tabId);
    }
  });
});

// Handle messages from content scripts
chrome.runtime.onMessage.addListener((message, sender) => {
  const senderTabId = sender.tab?.id;

  // Track Sp00ky tabs
  if (message.type === 'SP00KY_DETECTED' && senderTabId) {
    sp00kyTabs.set(senderTabId, {
      url: sender.tab?.url,
      title: sender.tab?.title,
    });
    reportTabsToBridge();
  }

  // Handle bridge responses (from page-script via content script)
  if (message.type === 'SP00KY_BRIDGE_RESPONSE' && message.requestId) {
    const bridgeId = message.requestId.replace('bridge-', '');
    pendingBridgeRequests.delete(message.requestId);

    if (message.success) {
      sendBridgeResponse(bridgeId, message.data ?? { success: true });
    } else {
      sendBridgeError(bridgeId, -32000, message.error || 'Unknown error');
    }
    // Don't return early - let it also forward to devtools panel if connected
  }

  // Handle query responses for bridge
  if (message.type === 'SP00KY_QUERY_RESPONSE' && message.requestId?.startsWith('bridge-')) {
    const bridgeId = message.requestId.replace('bridge-', '');
    pendingBridgeRequests.delete(message.requestId);

    if (message.success) {
      sendBridgeResponse(bridgeId, { success: true, data: message.data });
    } else {
      sendBridgeError(bridgeId, -32000, message.error || 'Query failed');
    }
  }

  // Handle state responses for bridge getState requests
  if (message.type === 'SP00KY_STATE_CHANGED' && senderTabId) {
    // Check if any pending bridge getState requests match this tab
    for (const [reqId, reqTabId] of pendingBridgeRequests) {
      if (reqTabId === senderTabId && !reqId.startsWith('bridge-')) {
        pendingBridgeRequests.delete(reqId);
        sendBridgeResponse(reqId, message.state);
      }
    }
  }

  // Forward state updates to the appropriate devtools panel
  if (senderTabId) {
    if (connections.has(senderTabId)) {
      const port = connections.get(senderTabId);
      console.log(
        '[DevTools Background] Forwarding content message to panel. Type:',
        message.type,
        'Tab:',
        senderTabId
      );
      port?.postMessage(message);
    } else {
      console.log(
        '[DevTools Background] NO CONNECTION found for tab',
        senderTabId,
        'Active connections:',
        Array.from(connections.keys())
      );
    }
  } else {
    console.warn('[DevTools Background] Message from unknown sender (no tab id)', sender);
  }
});

// Detect when tabs are updated
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === 'complete' && connections.has(tabId)) {
    // Notify the devtools panel that the page has been reloaded
    const port = connections.get(tabId);
    port?.postMessage({ type: 'PAGE_RELOADED' });
  }
});

// Clean up sp00ky tabs when tabs are closed
chrome.tabs.onRemoved.addListener((tabId) => {
  if (sp00kyTabs.has(tabId)) {
    sp00kyTabs.delete(tabId);
    reportTabsToBridge();
  }
});
