# Spooky DevTools

A Chrome DevTools extension for debugging and inspecting Spooky state in your applications.

## Features

- Detect Spooky instances on any webpage
- View all registered stores
- Inspect store state in real-time
- See subscriber counts and sync status
- Auto-refresh on page navigation

## Development

### Prerequisites

- Node.js 18+
- pnpm

### Setup

```bash
# Install dependencies
pnpm install

# Build the extension
pnpm build

# Or run in development mode with watch
pnpm dev
```

### Loading the Extension in Chrome

1. Build the extension using `pnpm build`
2. Open Chrome and navigate to `chrome://extensions/`
3. Enable "Developer mode" (toggle in top-right corner)
4. Click "Load unpacked"
5. Select the `packages/devtools/dist` directory
6. The Spooky DevTools extension should now be loaded

### Using the Extension

1. Open Chrome DevTools (F12 or right-click > Inspect)
2. Look for the "Spooky" tab in the DevTools
3. Navigate to a page that uses Spooky
4. The extension will automatically detect Spooky and display available stores
5. Click on any store in the sidebar to view its current state

## Extension Structure

```
packages/devtools/
├── src/
│   ├── devtools.ts      # DevTools page - creates the panel
│   ├── panel.ts         # Panel UI and logic
│   ├── background.ts    # Background service worker
│   └── content.ts       # Content script - detects Spooky
├── public/
│   ├── devtools.html    # DevTools page HTML
│   ├── panel.html       # Panel UI HTML
│   └── icons/           # Extension icons
├── manifest.json        # Chrome extension manifest
├── vite.config.ts       # Build configuration
└── package.json
```

## How It Works

1. **Content Script** (`content.ts`): Injected into every page, checks for `window.__SPOOKY__`
2. **Background Script** (`background.ts`): Handles communication between content scripts and DevTools
3. **DevTools Page** (`devtools.ts`): Creates the Spooky panel in Chrome DevTools
4. **Panel** (`panel.ts`): Displays the UI for viewing stores and state

## TODO / Future Enhancements

- [ ] Add time-travel debugging
- [ ] Show state diffs when stores update
- [ ] Display sync operations and network activity
- [ ] Add ability to modify state directly from DevTools
- [ ] Show component tree that uses each store
- [ ] Export/import state snapshots
- [ ] Performance profiling for store updates
- [ ] Search and filter stores
- [ ] Dark/light theme toggle
- [ ] Better icons and branding

## Requirements for Spooky Apps

For the DevTools to detect your Spooky application, you need to expose Spooky on the window object during development:

```typescript
// In your app's initialization
if (import.meta.env.DEV) {
  (window as any).__SPOOKY__ = {
    version: '1.0.0',
    stores: yourStoresMap,
    // Optional: hook for updates
    onUpdate: (callback: () => void) => {
      // Register callback to be called on state changes
    },
  };

  // Optionally dispatch an event
  window.dispatchEvent(new Event('spooky:init'));
}
```

## License

Part of the Spooky monorepo.
