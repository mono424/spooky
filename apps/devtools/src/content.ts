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
    script.remove();
  };
  (document.head || document.documentElement).appendChild(script);
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
chrome.runtime.onMessage.addListener((message) => {
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
