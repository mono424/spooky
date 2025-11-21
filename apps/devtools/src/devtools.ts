// DevTools page script - creates the panel
chrome.devtools.panels.create(
  'Spooky',
  'icons/icon48.png',
  'panel.html',
  () => {
    console.log('Spooky DevTools panel created');
  }
);
