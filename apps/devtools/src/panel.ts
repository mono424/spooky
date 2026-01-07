// Panel script - handles the UI and communication

interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

interface ActiveQuery {
  queryHash: number;
  status: 'initializing' | 'active' | 'updating' | 'destroyed';
  createdAt: number;
  lastUpdate: number;
  updateCount: number;
  dataSize?: number;
  query?: string;
  variables?: Record<string, unknown>;
}

interface AuthState {
  authenticated: boolean;
  userId?: string;
  timestamp?: number;
}

interface DatabaseState {
  tables: string[];
  tableData: Record<string, Record<string, unknown>[]>;
}

interface DevToolsState {
  eventsHistory: DevToolsEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: AuthState;
  version: string;
  database?: DatabaseState;
}

interface SpookyData {
  detected: boolean;
  version?: string;
  state?: DevToolsState | null;
}

let currentData: SpookyData | null = null;
let selectedQueryHash: number | null = null;
let selectedTableName: string | null = null;

// Get the inspected tab ID
const tabId = chrome.devtools.inspectedWindow.tabId;

// Create a connection to the background script
const backgroundConnection = chrome.runtime.connect({
  name: 'spooky-devtools-panel',
});

// Detect and apply Chrome DevTools theme
function detectAndApplyTheme() {
  function applyTheme(theme: 'light' | 'dark') {
    document.documentElement.setAttribute('data-theme', theme);
  }

  // Method 1: Try to get themeName from chrome.devtools.panels (if available)
  try {
    const themeName = (chrome.devtools.panels as any).themeName;
    if (themeName) {
      const isDark = themeName === 'dark' || themeName === 'default';
      applyTheme(isDark ? 'dark' : 'light');

      // Listen for theme changes if API is available
      if (typeof (chrome.devtools.panels as any).onThemeChanged !== 'undefined') {
        (chrome.devtools.panels as any).onThemeChanged.addListener((newThemeName: string) => {
          const isDarkTheme = newThemeName === 'dark' || newThemeName === 'default';
          applyTheme(isDarkTheme ? 'dark' : 'light');
        });
      }
      return;
    }
  } catch (e) {
    // themeName API might not be available
  }

  // Method 2: Detect theme by checking the computed background color
  // Chrome DevTools styles the body with a background color we can detect
  function detectThemeFromBackground() {
    // Wait for body to be styled by Chrome DevTools
    setTimeout(() => {
      try {
        const bodyBg = window.getComputedStyle(document.body).backgroundColor;
        const rgbMatch = bodyBg.match(/\d+/g);

        if (rgbMatch && rgbMatch.length >= 3) {
          const r = parseInt(rgbMatch[0]);
          const g = parseInt(rgbMatch[1]);
          const b = parseInt(rgbMatch[2]);
          // Calculate relative luminance (per WCAG)
          const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
          applyTheme(luminance > 0.5 ? 'light' : 'dark');
        } else {
          // Fallback to system preference
          const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
          applyTheme(prefersDark ? 'dark' : 'light');
        }
      } catch (e) {
        // Fallback to system preference
        const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
        applyTheme(prefersDark ? 'dark' : 'light');
      }
    }, 100);
  }

  // Initial detection
  detectThemeFromBackground();

  // Listen for system theme changes
  const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
  mediaQuery.addEventListener('change', (e) => {
    // Only apply if we couldn't detect DevTools theme directly
    try {
      if (!(chrome.devtools.panels as any).themeName) {
        applyTheme(e.matches ? 'dark' : 'light');
      }
    } catch (err) {
      applyTheme(e.matches ? 'dark' : 'light');
    }
  });

  // Periodically check for theme changes (fallback)
  // This helps catch theme changes that might not trigger other events
  let lastTheme: string | null = null;
  setInterval(() => {
    try {
      const bodyBg = window.getComputedStyle(document.body).backgroundColor;
      const rgbMatch = bodyBg.match(/\d+/g);

      if (rgbMatch && rgbMatch.length >= 3) {
        const r = parseInt(rgbMatch[0]);
        const g = parseInt(rgbMatch[1]);
        const b = parseInt(rgbMatch[2]);
        const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
        const detectedTheme = luminance > 0.5 ? 'light' : 'dark';

        if (lastTheme !== detectedTheme) {
          lastTheme = detectedTheme;
          applyTheme(detectedTheme);
        }
      }
    } catch (e) {
      // Ignore errors
    }
  }, 2000);
}

// Initialize the panel
function initPanel() {
  // Ensure default theme is set first
  if (!document.documentElement.hasAttribute('data-theme')) {
    document.documentElement.setAttribute('data-theme', 'dark');
  }

  // Detect and apply Chrome DevTools theme
  detectAndApplyTheme();

  const refreshBtn = document.getElementById('refresh-btn');
  const clearEventsBtn = document.getElementById('clear-events-btn');

  refreshBtn?.addEventListener('click', refreshState);
  clearEventsBtn?.addEventListener('click', clearEventsHistory);

  // Set up tab switching
  setupTabs();

  // Tell the background script which tab we're inspecting
  backgroundConnection.postMessage({
    name: 'init',
    tabId: tabId,
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

    if (message.type === 'SPOOKY_TABLE_DATA_RESPONSE') {
      console.log('SPOOKY_TABLE_DATA_RESPONSE received:', message);
      const { tableName, data } = message;
      if (tableName && data) {
        updateTableData(tableName, data);
      }
    }
  });

  // Initial state fetch
  refreshState();
}

// Fetch the current Spooky state
function refreshState() {
  chrome.devtools.inspectedWindow.eval(`(${detectSpooky.toString()})()`, (result, isException) => {
    if (isException) {
      console.error('Error detecting Spooky:', isException);
      updateUI({ detected: false });
    } else if (result && typeof result === 'object' && 'detected' in result) {
      updateUI(result as SpookyData);
    } else {
      updateUI({ detected: false });
    }
  });
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
    if (data.state.database) {
      updateDatabaseTables(data.state.database.tables);
      if (selectedTableName && data.state.database.tableData[selectedTableName]) {
        updateTableData(selectedTableName, data.state.database.tableData[selectedTableName]);
      }
    }
  } else {
    console.log('No state available, showing empty state');
    // Show empty states
    updateEventsHistory([]);
    updateActiveQueries({});
    updateAuthInfo({ authenticated: false });
    updateDatabaseTables([]);
  }
}

function updateStatus(data: SpookyData) {
  const statusEl = document.getElementById('status');
  if (statusEl) {
    const statusDot = statusEl.querySelector('.status-dot');
    const statusText = statusEl.querySelector('span:last-child');

    if (statusDot && statusText) {
      if (data.detected) {
        statusDot.className = 'status-dot active';
        const authStatus = data.state?.auth.authenticated ? 'Authenticated' : 'Not authenticated';
        statusText.textContent = `Spooky detected ${
          data.version ? `(v${data.version})` : ''
        } • ${authStatus}`;
      } else {
        statusDot.className = 'status-dot inactive';
        statusText.textContent = 'Spooky not detected on this page';
      }
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
  const queriesListContentEl = document.getElementById('queries-list-content');
  if (!queriesListContentEl) return;

  const queryArray = Object.values(queries);

  if (queryArray.length === 0) {
    queriesListContentEl.innerHTML = '<div class="empty-state">No active queries</div>';
    selectedQueryHash = null;
    hideQueryDetail();
    return;
  }

  queriesListContentEl.innerHTML = queryArray
    .map((query) => {
      const age = Math.floor((Date.now() - query.createdAt) / 1000);
      const isSelected = selectedQueryHash === query.queryHash;
      const queryPreview = query.query
        ? query.query.substring(0, 50) + (query.query.length > 50 ? '...' : '')
        : '';
      return `
        <div class="query-item ${
          isSelected ? 'selected' : ''
        }" data-query-hash="${query.queryHash}">
          <div class="query-header">
            <span class="query-hash">Query #${query.queryHash}</span>
            <span class="query-status status-${query.status}">${query.status}</span>
          </div>
          ${queryPreview ? `<div class="query-preview">${queryPreview}</div>` : ''}
          <div class="query-meta">
            Updates: ${query.updateCount} •
            Size: ${query.dataSize ?? '?'} •
            Age: ${age}s
          </div>
        </div>
      `;
    })
    .join('');

  // Add click handlers to query items
  queriesListContentEl.querySelectorAll('.query-item').forEach((item) => {
    item.addEventListener('click', () => {
      const queryHash = parseInt((item as HTMLElement).dataset.queryHash || '0');
      selectQuery(queryHash, queries);
    });
  });

  // If a query was selected, update the detail view
  if (selectedQueryHash && queries[selectedQueryHash]) {
    showQueryDetail(queries[selectedQueryHash]);
  }
}

function selectQuery(queryHash: number, queries: Record<number, ActiveQuery>) {
  selectedQueryHash = queryHash;
  const query = queries[queryHash];

  if (query) {
    showQueryDetail(query);
    // Update the selected state in the list
    const queriesListContentEl = document.getElementById('queries-list-content');
    if (queriesListContentEl) {
      queriesListContentEl.querySelectorAll('.query-item').forEach((item) => {
        if (parseInt((item as HTMLElement).dataset.queryHash || '0') === queryHash) {
          item.classList.add('selected');
        } else {
          item.classList.remove('selected');
        }
      });
    }
  }
}

function showQueryDetail(query: ActiveQuery) {
  const detailEl = document.getElementById('query-detail');
  if (!detailEl) {
    console.error('Query detail element not found');
    return;
  }

  const age = Math.floor((Date.now() - query.createdAt) / 1000);
  const lastUpdate = Math.floor((Date.now() - query.lastUpdate) / 1000);
  const createdTime = new Date(query.createdAt).toLocaleString();
  const lastUpdateTime = new Date(query.lastUpdate).toLocaleString();

  console.log('Showing query detail for:', query.queryHash);

  detailEl.innerHTML = `
    <div class="detail-header">
      <h3>Query #${query.queryHash}</h3>
    </div>
    <div class="detail-content">
      <div class="detail-section">
        <div class="detail-label">Status</div>
        <div class="detail-value">
          <span class="query-status status-${query.status}">${query.status}</span>
        </div>
      </div>

      <div class="detail-section">
        <div class="detail-label">Query Hash</div>
        <div class="detail-value mono">${query.queryHash}</div>
      </div>

      ${
        query.query
          ? `
      <div class="detail-section">
        <div class="detail-label">Query</div>
        <div class="detail-value">
          <pre class="query-code">${query.query}</pre>
        </div>
      </div>
      `
          : ''
      }

      ${
        query.variables && Object.keys(query.variables).length > 0
          ? `
      <div class="detail-section">
        <div class="detail-label">Variables</div>
        <div class="detail-value">
          <pre class="query-code">${JSON.stringify(query.variables, null, 2)}</pre>
        </div>
      </div>
      `
          : ''
      }

      <div class="detail-section">
        <div class="detail-label">Created</div>
        <div class="detail-value">${createdTime} (${age}s ago)</div>
      </div>

      <div class="detail-section">
        <div class="detail-label">Last Update</div>
        <div class="detail-value">${lastUpdateTime} (${lastUpdate}s ago)</div>
      </div>

      <div class="detail-section">
        <div class="detail-label">Update Count</div>
        <div class="detail-value">${query.updateCount}</div>
      </div>

      <div class="detail-section">
        <div class="detail-label">Data Size</div>
        <div class="detail-value">${query.dataSize ?? 'unknown'} records</div>
      </div>

      <div class="detail-section">
        <div class="detail-label">Performance</div>
        <div class="detail-value">
          ${
            query.updateCount > 0
              ? `${(query.updateCount / (age || 1)).toFixed(2)} updates/sec`
              : 'No updates yet'
          }
        </div>
      </div>
    </div>
  `;
}

function hideQueryDetail() {
  const detailEl = document.getElementById('query-detail');
  if (detailEl) {
    detailEl.innerHTML = '';
  }

  selectedQueryHash = null;

  // Remove selection from all query items
  const queriesListContentEl = document.getElementById('queries-list-content');
  if (queriesListContentEl) {
    queriesListContentEl.querySelectorAll('.query-item').forEach((item) => {
      item.classList.remove('selected');
    });
  }
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

function updateDatabaseTables(tables: string[]) {
  console.log('updateDatabaseTables called with:', tables);
  const tablesListEl = document.getElementById('tables-list');
  if (!tablesListEl) {
    console.error('tables-list element not found');
    return;
  }

  if (tables.length === 0) {
    tablesListEl.innerHTML = '<div class="empty-state">No tables available</div>';
    return;
  }

  tablesListEl.innerHTML = tables
    .map((tableName) => {
      const isSelected = selectedTableName === tableName;
      return `
        <div class="table-item ${isSelected ? 'selected' : ''}" data-table-name="${tableName}">
          ${tableName}
        </div>
      `;
    })
    .join('');

  console.log('Added table items, now adding click handlers');

  // Add click handlers to table items
  tablesListEl.querySelectorAll('.table-item').forEach((item) => {
    item.addEventListener('click', () => {
      const tableName = (item as HTMLElement).dataset.tableName;
      console.log('Table item clicked:', tableName);
      if (tableName) {
        selectTable(tableName);
      }
    });
  });
}

function selectTable(tableName: string) {
  console.log('selectTable called with:', tableName);
  selectedTableName = tableName;

  // Update selected state in the list
  const tablesListEl = document.getElementById('tables-list');
  if (tablesListEl) {
    tablesListEl.querySelectorAll('.table-item').forEach((item) => {
      if ((item as HTMLElement).dataset.tableName === tableName) {
        item.classList.add('selected');
        console.log('Added selected class to:', tableName);
      } else {
        item.classList.remove('selected');
      }
    });
  }

  // Fetch table data
  fetchTableData(tableName);
}

function fetchTableData(tableName: string) {
  // Use a message-based approach since promises don't work well with eval
  chrome.devtools.inspectedWindow.eval(
    `(async function() {
      try {
        if (window.__SPOOKY__ && window.__SPOOKY__.getTableData) {
          console.log('Fetching table data for:', "${tableName}");
          const data = await window.__SPOOKY__.getTableData("${tableName}");
          console.log('Got table data:', data);

          // Post message back to devtools
          window.postMessage({
            type: 'SPOOKY_TABLE_DATA_RESPONSE',
            source: 'spooky-devtools-page',
            tableName: "${tableName}",
            data: data
          }, '*');

          return { success: true, count: data?.length || 0 };
        }
        console.warn('window.__SPOOKY__.getTableData not available');
        return { success: false, error: 'API not available' };
      } catch (error) {
        console.error('Error fetching table data:', error);
        return { success: false, error: error.message };
      }
    })()`,
    (result: any, isException: any) => {
      console.log('Eval completed:', { tableName, result, isException });
    }
  );
}

function updateTableData(tableName: string, data: Record<string, unknown>[]) {
  console.log('updateTableData called with:', {
    tableName,
    dataLength: data?.length,
    data,
  });
  const tableDataEl = document.getElementById('table-data');

  if (!tableDataEl) {
    console.error('table-data or table-name-header element not found');
    return;
  }

  // Safety check for data
  if (!data || !Array.isArray(data) || data.length === 0) {
    console.log('No data to display');
    tableDataEl.innerHTML = '<div class="empty-state">No data in this table</div>';
    return;
  }

  try {
    // Get all unique columns from the data
    const columns = new Set<string>();
    data.forEach((row) => {
      if (row && typeof row === 'object') {
        Object.keys(row).forEach((key) => columns.add(key));
      }
    });
    const columnArray = Array.from(columns).sort((a, b) => {
      if (a === 'id') return -1;
      if (b === 'id') return 1;
      return a.localeCompare(b);
    });

    if (columnArray.length === 0) {
      tableDataEl.innerHTML = '<div class="empty-state">No columns found in data</div>';
      return;
    }

    // Build table header row
    const headerCells = columnArray.map((col) => `<th>${escapeHtml(col)}</th>`).join('');

    // Build table data rows
    const rowsHtml = data
      .map((row) => {
        const cellsHtml = columnArray
          .map((col) => {
            const value = row[col];
            let displayValue = '';
            if (value === null || value === undefined) {
              displayValue = '<em>null</em>';
            } else if (typeof value === 'object') {
              try {
                displayValue = escapeHtml(JSON.stringify(value));
              } catch (e) {
                displayValue = escapeHtml('[Object]');
              }
            } else {
              displayValue = escapeHtml(String(value));
            }
            return `<td title="${displayValue}">${displayValue}</td>`;
          })
          .join('');
        return `<tr>${cellsHtml}</tr>`;
      })
      .join('');

    // Build complete table with thead
    const tableHtml = `
      <table class="data-table">
        <thead>
          <tr>${headerCells}</tr>
        </thead>
        <tbody>
          ${rowsHtml}
        </tbody>
      </table>
    `;

    tableDataEl.innerHTML = tableHtml;
  } catch (error) {
    console.error('Error rendering table:', error);
    tableDataEl.innerHTML = '<div class="empty-state">Error rendering table data</div>';
  }
}

function escapeHtml(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// Initialize when the DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initPanel);
} else {
  initPanel();
}
