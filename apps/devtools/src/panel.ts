// Panel script - handles the UI and communication
interface SpookyStore {
  name: string;
  state: any;
  subscribers: number;
  synced: boolean;
}

interface SpookyData {
  detected: boolean;
  version?: string;
  stores: SpookyStore[];
}

let currentData: SpookyData | null = null;

// Get the inspected tab ID
const tabId = chrome.devtools.inspectedWindow.tabId;

// Initialize the panel
function initPanel() {
  const refreshBtn = document.getElementById('refresh-btn');
  refreshBtn?.addEventListener('click', refreshState);

  // Initial state fetch
  refreshState();

  // Listen for state updates from the background script
  chrome.runtime.onMessage.addListener((message) => {
    if (message.type === 'SPOOKY_STATE_UPDATE') {
      updateUI(message.data);
    }
  });
}

// Fetch the current Spooky state
function refreshState() {
  chrome.devtools.inspectedWindow.eval(
    `(${detectSpooky.toString()})()`,
    (result, isException) => {
      if (isException) {
        console.error('Error detecting Spooky:', isException);
        updateUI({ detected: false, stores: [] });
      } else if (result && typeof result === 'object' && 'detected' in result) {
        updateUI(result as SpookyData);
      } else {
        updateUI({ detected: false, stores: [] });
      }
    }
  );
}

// Function that runs in the page context to detect Spooky
function detectSpooky() {
  try {
    // Check if Spooky is available on the window object
    const spooky = (window as any).__SPOOKY__;

    if (!spooky) {
      return { detected: false, stores: [] };
    }

    // Collect information about stores
    const stores = [];
    if (spooky.stores && typeof spooky.stores === 'object') {
      for (const [name, store] of Object.entries(spooky.stores)) {
        const storeData = store as any;
        stores.push({
          name,
          state: storeData.state || storeData.get?.() || {},
          subscribers: storeData.subscribers?.size || 0,
          synced: storeData.synced || false,
        });
      }
    }

    return {
      detected: true,
      version: spooky.version || 'unknown',
      stores,
    };
  } catch (error) {
    console.error('Error in detectSpooky:', error);
    return { detected: false, stores: [] };
  }
}

// Update the UI with Spooky data
function updateUI(data: SpookyData) {
  currentData = data;

  // Update status
  const statusEl = document.getElementById('status');
  if (statusEl) {
    if (data.detected) {
      statusEl.className = 'status active';
      statusEl.innerHTML = `<strong>Status:</strong> Spooky detected ${data.version ? `(v${data.version})` : ''}`;
    } else {
      statusEl.className = 'status inactive';
      statusEl.innerHTML = '<strong>Status:</strong> Spooky not detected on this page';
    }
  }

  // Update store list
  const storeListEl = document.getElementById('store-list');
  if (storeListEl) {
    if (data.stores.length === 0) {
      storeListEl.innerHTML = '<li class="empty-state">No stores detected</li>';
    } else {
      storeListEl.innerHTML = data.stores
        .map((store, index) => `
          <li class="store-item" data-index="${index}">
            <div class="store-name">${store.name}</div>
            <div class="store-meta">
              ${store.subscribers} subscriber(s) • ${store.synced ? 'Synced' : 'Local'}
            </div>
          </li>
        `)
        .join('');

      // Add click handlers
      storeListEl.querySelectorAll('.store-item').forEach((item) => {
        item.addEventListener('click', () => {
          const index = parseInt((item as HTMLElement).dataset.index || '0');
          showStoreState(data.stores[index]);
        });
      });
    }
  }
}

// Show the state of a selected store
function showStoreState(store: SpookyStore) {
  const stateViewerEl = document.getElementById('state-viewer');
  if (stateViewerEl) {
    stateViewerEl.innerHTML = `<pre>${JSON.stringify(store.state, null, 2)}</pre>`;
  }
}

// Initialize when the DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initPanel);
} else {
  initPanel();
}
