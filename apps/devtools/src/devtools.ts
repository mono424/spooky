// DevTools page script - creates the panel
chrome.devtools.panels.create('Sp00ky', 'icons/icon48.png', 'panel.html', () => {
  console.log('Sp00ky DevTools panel created');
});
