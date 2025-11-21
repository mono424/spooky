// Content script - injects a script into the page to access Spooky
// and communicates with the background script

console.log('Spooky DevTools content script loaded');

// Inject a script into the page context to access window.__SPOOKY__
// Using external file to avoid CSP violations with inline scripts
function injectPageScript() {
  const script = document.createElement('script');
  script.src = chrome.runtime.getURL('page-script.js');
  script.onload = function() {
    // Remove script tag after execution to keep DOM clean
    try {
      script.remove();
    } catch (e) {
      // Ignore errors if script is already removed
      console.warn('[DevTools] Script removal failed:', e);
    }
  };
  script.onerror = function(error) {
    console.error('[DevTools] Failed to load page-script.js:', error);
  };
  (document.head || document.documentElement).appendChild(script);
}

// Listen for messages from the injected script
window.addEventListener('message', (event) => {
  // Only accept messages from the same window
  if (event.source !== window) return;

  // Only handle messages from our injected script
  if (event.data.source !== 'spooky-devtools-page') return;

  // Forward to background script with all relevant data
  try {
    chrome.runtime.sendMessage({
      type: event.data.type,
      data: event.data.data,
      state: event.data.state, // Include state for SPOOKY_STATE_CHANGED messages
      tableName: event.data.tableName, // Include tableName for table data responses
    }).catch((error) => {
      // Silently ignore "Extension context invalidated" errors (happens during dev reloads)
      if (!error.message?.includes('Extension context invalidated')) {
        console.warn('[DevTools] Failed to send message to background:', error);
      }
    });
  } catch (error) {
    // Extension was reloaded, runtime is no longer available
    // This is normal during development, silently ignore
  }
});

// Listen for messages from the background script/devtools
chrome.runtime.onMessage.addListener((message, _sender, _sendResponse) => {
  if (message.type === 'GET_SPOOKY_STATE') {
    // Request state from the page
    window.postMessage({
      type: 'GET_STATE',
      source: 'spooky-devtools-content'
    }, '*');
  }
  // Return true to indicate we may send a response asynchronously
  return true;
});

// Inject the page script
injectPageScript();
