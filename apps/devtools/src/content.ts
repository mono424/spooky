// Content script - injects a script into the page to access Sp00ky
// and communicates with the background script

console.log('Sp00ky DevTools content script loaded');

// Inject a script into the page context to access window.__SP00KY__
// Using external file to avoid CSP violations with inline scripts
function injectPageScript() {
  const script = document.createElement('script');
  script.src = chrome.runtime.getURL('page-script.js');
  script.onload = function () {
    // Remove script tag after execution to keep DOM clean
    try {
      script.remove();
    } catch (e) {
      // Ignore errors if script is already removed
      console.warn('[DevTools] Script removal failed:', e);
    }
  };
  script.onerror = function (error) {
    console.error('[DevTools] Failed to load page-script.js:', error);
  };
  (document.head || document.documentElement).appendChild(script);
}

// Listen for messages from the injected script
window.addEventListener('message', (event) => {
  // Only accept messages from the same window
  if (event.source !== window) return;

  // Only handle messages from our injected script
  if (event.data.source !== 'sp00ky-devtools-page') return;

  // Debug logging - Log EVERYTHING to debug connection
  console.log('[DevTools Content Script] Forwarding message:', event.data.type);

  // Forward to background script with all relevant data
  try {
    chrome.runtime
      .sendMessage({
        ...event.data,
      })
      .catch((error) => {
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
  if (message.type === 'GET_SP00KY_STATE') {
    // Request state from the page
    window.postMessage(
      {
        type: 'GET_STATE',
        source: 'sp00ky-devtools-content',
      },
      '*'
    );
  } else if (message.type === 'RUN_QUERY') {
    // Dispatch event for page-script.ts to handle
    window.dispatchEvent(
      new CustomEvent('SP00KY_RUN_QUERY', {
        detail: message.payload,
      })
    );
  } else if (message.type === 'GET_TABLE_DATA') {
    window.dispatchEvent(
      new CustomEvent('SP00KY_GET_TABLE_DATA', {
        detail: message.payload,
      })
    );
  } else if (message.type === 'UPDATE_TABLE_ROW') {
    window.dispatchEvent(
      new CustomEvent('SP00KY_UPDATE_TABLE_ROW', {
        detail: message.payload,
      })
    );
  } else if (message.type === 'DELETE_TABLE_ROW') {
    window.dispatchEvent(
      new CustomEvent('SP00KY_DELETE_TABLE_ROW', {
        detail: message.payload,
      })
    );
  } else if (message.type === 'CLEAR_HISTORY') {
    window.dispatchEvent(
      new CustomEvent('SP00KY_CLEAR_HISTORY', {
        detail: message.payload,
      })
    );
  }
  // Return true to indicate we may send a response asynchronously
  return true;
});

// Inject the page script
injectPageScript();
