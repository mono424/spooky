// DevTools page script - creates the panel
chrome.devtools.panels.create(
  'Spooky',
  'icons/icon48.png',
  'panel.html',
  (panel) => {
    console.log('Spooky DevTools panel created');
  }
);
