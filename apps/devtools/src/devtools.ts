// DevTools page script - creates the panel
chrome.devtools.panels.create('00', 'icons/icon48.png', 'panel.html', () => {
  console.log('00 DevTools panel created');
});
