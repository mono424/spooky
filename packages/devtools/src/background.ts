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
      connections.set(tabId, port);
    }

    // Forward messages to the content script
    if (tabId) {
      chrome.tabs.sendMessage(tabId, message);
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
  if (sender.tab?.id && connections.has(sender.tab.id)) {
    const port = connections.get(sender.tab.id);
    port?.postMessage(message);
  }
});

// Detect when tabs are updated
chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === 'complete' && connections.has(tabId)) {
    // Notify the devtools panel that the page has been reloaded
    const port = connections.get(tabId);
    port?.postMessage({ type: 'PAGE_RELOADED' });
  }
});
