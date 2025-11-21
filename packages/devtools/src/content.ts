// Content script - injects a script into the page to access Spooky
// and communicates with the background script

console.log('Spooky DevTools content script loaded');

// Inject a script into the page context to access window.__SPOOKY__
function injectPageScript() {
  const script = document.createElement('script');
  script.textContent = `
    (function() {
      // Hook into Spooky if it exists
      function checkForSpooky() {
        if (window.__SPOOKY__) {
          console.log('Spooky detected by DevTools');

          // Send initial detection message
          window.postMessage({
            type: 'SPOOKY_DETECTED',
            source: 'spooky-devtools-page',
            data: {
              version: window.__SPOOKY__.version,
              detected: true
            }
          }, '*');

          // Hook into store updates if possible
          if (window.__SPOOKY__.onUpdate) {
            window.__SPOOKY__.onUpdate(() => {
              window.postMessage({
                type: 'SPOOKY_STATE_CHANGED',
                source: 'spooky-devtools-page'
              }, '*');
            });
          }

          return true;
        }
        return false;
      }

      // Try immediately
      if (!checkForSpooky()) {
        // If not found, try again after a short delay
        setTimeout(checkForSpooky, 100);
        setTimeout(checkForSpooky, 500);
        setTimeout(checkForSpooky, 1000);
      }

      // Also listen for custom event in case Spooky loads later
      window.addEventListener('spooky:init', () => {
        checkForSpooky();
      });
    })();
  `;

  (document.head || document.documentElement).appendChild(script);
  script.remove();
}

// Listen for messages from the injected script
window.addEventListener('message', (event) => {
  // Only accept messages from the same window
  if (event.source !== window) return;

  // Only handle messages from our injected script
  if (event.data.source !== 'spooky-devtools-page') return;

  // Forward to background script
  chrome.runtime.sendMessage({
    type: event.data.type,
    data: event.data.data
  });
});

// Listen for messages from the background script/devtools
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === 'GET_SPOOKY_STATE') {
    // Request state from the page
    window.postMessage({
      type: 'GET_STATE',
      source: 'spooky-devtools-content'
    }, '*');
  }
});

// Inject the page script
injectPageScript();
