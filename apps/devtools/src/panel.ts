// Panel script - handles the UI and communication

interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

interface ActiveQuery {
  queryHash: number;
  status: "initializing" | "active" | "updating" | "destroyed";
  createdAt: number;
  lastUpdate: number;
  updateCount: number;
  dataSize?: number;
}

interface AuthState {
  authenticated: boolean;
  userId?: string;
  timestamp?: number;
}

interface DevToolsState {
  eventsHistory: DevToolsEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: AuthState;
  version: string;
}

interface SpookyData {
  detected: boolean;
  version?: string;
  state?: DevToolsState | null;
}

let currentData: SpookyData | null = null;

// Get the inspected tab ID
const tabId = chrome.devtools.inspectedWindow.tabId;

// Create a connection to the background script
const backgroundConnection = chrome.runtime.connect({
  name: 'spooky-devtools-panel'
});

// Initialize the panel
function initPanel() {
  const refreshBtn = document.getElementById('refresh-btn');
  const clearEventsBtn = document.getElementById('clear-events-btn');

  refreshBtn?.addEventListener('click', refreshState);
  clearEventsBtn?.addEventListener('click', clearEventsHistory);

  // Set up tab switching
  setupTabs();

  // Tell the background script which tab we're inspecting
  backgroundConnection.postMessage({
    name: 'init',
    tabId: tabId
  });

  // Listen for state updates from the background script via the port
  backgroundConnection.onMessage.addListener((message) => {
    console.log('Panel received message:', message);

    if (message.type === 'SPOOKY_DETECTED') {
      console.log('SPOOKY_DETECTED message received with data:', message.data);
      updateUI(message.data || { detected: true, version: message.data?.version });
    }

    if (message.type === 'SPOOKY_STATE_CHANGED') {
      console.log('SPOOKY_STATE_CHANGED message received with state:', message.state);
      // State is included in the message from devtools-service
      if (message.state) {
        updateUI({ detected: true, state: message.state });
      }
    }
  });

  // Initial state fetch
  refreshState();
}

// Fetch the current Spooky state
function refreshState() {
  chrome.devtools.inspectedWindow.eval(
    `(${detectSpooky.toString()})()`,
    (result, isException) => {
      if (isException) {
        console.error('Error detecting Spooky:', isException);
        updateUI({ detected: false });
      } else if (result && typeof result === 'object' && 'detected' in result) {
        updateUI(result as SpookyData);
      } else {
        updateUI({ detected: false });
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
      return { detected: false };
    }

    return {
      detected: true,
      version: spooky.version || 'unknown',
      state: spooky.getState ? spooky.getState() : null,
    };
  } catch (error) {
    console.error('Error in detectSpooky:', error);
    return { detected: false };
  }
}

// Update the UI with Spooky data
function updateUI(data: SpookyData) {
  console.log('updateUI called with data:', data);
  currentData = data;

  // Update status
  updateStatus(data);

  // Update content based on current tab
  if (data.state) {
    console.log('Updating UI with state:', data.state);
    updateEventsHistory(data.state.eventsHistory);
    updateActiveQueries(data.state.activeQueries);
    updateAuthInfo(data.state.auth);
  } else {
    console.log('No state available, showing empty state');
    // Show empty states
    updateEventsHistory([]);
    updateActiveQueries({});
    updateAuthInfo({ authenticated: false });
  }
}

function updateStatus(data: SpookyData) {
  const statusEl = document.getElementById('status');
  if (statusEl) {
    if (data.detected) {
      statusEl.className = 'status active';
      const authStatus = data.state?.auth.authenticated ? 'Authenticated' : 'Not authenticated';
      statusEl.innerHTML = `<strong>Status:</strong> Spooky detected ${data.version ? `(v${data.version})` : ''} • ${authStatus}`;
    } else {
      statusEl.className = 'status inactive';
      statusEl.innerHTML = '<strong>Status:</strong> Spooky not detected on this page';
    }
  }
}

function updateEventsHistory(events: DevToolsEvent[]) {
  const eventsListEl = document.getElementById('events-list');
  if (!eventsListEl) return;

  if (events.length === 0) {
    eventsListEl.innerHTML = '<div class="empty-state">No events recorded yet</div>';
    return;
  }

  // Reverse to show newest first
  const reversedEvents = [...events].reverse();
  eventsListEl.innerHTML = reversedEvents
    .map((event) => {
      const time = new Date(event.timestamp).toLocaleTimeString();
      return `
        <div class="event-item">
          <div class="event-header">
            <span class="event-type">${event.eventType}</span>
            <span class="event-time">${time}</span>
          </div>
          <div class="event-payload">
            <pre>${JSON.stringify(event.payload, null, 2)}</pre>
          </div>
        </div>
      `;
    })
    .join('');
}

function updateActiveQueries(queries: Record<number, ActiveQuery>) {
  const queriesListEl = document.getElementById('queries-list');
  if (!queriesListEl) return;

  const queryArray = Object.values(queries);

  if (queryArray.length === 0) {
    queriesListEl.innerHTML = '<div class="empty-state">No active queries</div>';
    return;
  }

  queriesListEl.innerHTML = queryArray
    .map((query) => {
      const age = Math.floor((Date.now() - query.createdAt) / 1000);
      const lastUpdate = Math.floor((Date.now() - query.lastUpdate) / 1000);
      return `
        <div class="query-item">
          <div class="query-header">
            <span class="query-hash">Query #${query.queryHash}</span>
            <span class="query-status status-${query.status}">${query.status}</span>
          </div>
          <div class="query-meta">
            Updates: ${query.updateCount} •
            Data size: ${query.dataSize ?? 'unknown'} •
            Age: ${age}s •
            Last update: ${lastUpdate}s ago
          </div>
        </div>
      `;
    })
    .join('');
}

function updateAuthInfo(auth: AuthState) {
  const authInfoEl = document.getElementById('auth-info');
  if (!authInfoEl) return;

  if (auth.authenticated) {
    const time = auth.timestamp ? new Date(auth.timestamp).toLocaleTimeString() : 'unknown';
    authInfoEl.innerHTML = `
      <div class="auth-status authenticated">
        <strong>Status:</strong> Authenticated<br>
        <strong>User ID:</strong> ${auth.userId || 'unknown'}<br>
        <strong>Time:</strong> ${time}
      </div>
    `;
  } else {
    authInfoEl.innerHTML = `
      <div class="auth-status not-authenticated">
        <strong>Status:</strong> Not authenticated
      </div>
    `;
  }
}

function setupTabs() {
  const tabBtns = document.querySelectorAll('.tab-btn');
  const tabContents = document.querySelectorAll('.tab-content');

  tabBtns.forEach((btn) => {
    btn.addEventListener('click', () => {
      const tabName = (btn as HTMLElement).dataset.tab;

      // Update active tab button
      tabBtns.forEach((b) => b.classList.remove('active'));
      btn.classList.add('active');

      // Show corresponding content
      tabContents.forEach((content) => {
        if ((content as HTMLElement).dataset.tab === tabName) {
          content.classList.add('active');
        } else {
          content.classList.remove('active');
        }
      });
    });
  });
}

function clearEventsHistory() {
  chrome.devtools.inspectedWindow.eval(
    `(function() {
      if (window.__SPOOKY__ && window.__SPOOKY__.clearHistory) {
        window.__SPOOKY__.clearHistory();
        return { success: true };
      }
      return { success: false };
    })()`,
    (result: any, isException) => {
      if (!isException && result?.success) {
        console.log('Events history cleared');
        refreshState();
      }
    }
  );
}

// Initialize when the DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initPanel);
} else {
  initPanel();
}
