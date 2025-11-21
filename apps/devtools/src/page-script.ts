// This script runs in the page context and has access to window.__SPOOKY__
(function() {
  // Hook into Spooky if it exists
  function checkForSpooky() {
    if ((window as any).__SPOOKY__) {
      console.log('Spooky detected by DevTools');

      // Send initial detection message
      window.postMessage({
        type: 'SPOOKY_DETECTED',
        source: 'spooky-devtools-page',
        data: {
          version: (window as any).__SPOOKY__.version,
          detected: true
        }
      }, '*');

      // Hook into store updates if possible
      if ((window as any).__SPOOKY__.onUpdate) {
        (window as any).__SPOOKY__.onUpdate(() => {
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
