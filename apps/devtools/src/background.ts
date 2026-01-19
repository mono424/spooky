// Background service worker for the extension
// Handles communication between content scripts and devtools panels

console.log('Spooky DevTools background script loaded');

// Keep track of active connections
const connections = new Map<number, chrome.runtime.Port>();

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
  // Forward state updates to the appropriate devtools panel
  if (sender.tab?.id) {
    if (connections.has(sender.tab.id)) {
      const port = connections.get(sender.tab.id);
      console.log(
        '[DevTools Background] Forwarding content message to panel. Type:',
        message.type,
        'Tab:',
        sender.tab.id
      );
      port?.postMessage(message);
    } else {
      console.log(
        '[DevTools Background] NO CONNECTION found for tab',
        sender.tab.id,
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
