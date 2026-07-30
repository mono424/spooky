import { Show } from 'solid-js';
import { DevToolsProvider, useDevTools } from './context/DevToolsContext';
import { useTheme } from './hooks/useTheme';
import { Tabs } from './components/Tabs';
import { EventsTab } from './components/events/EventsTab';
import { QueriesTab } from './components/queries/QueriesTab';
import { TimingTab } from './components/timing/TimingTab';
import { DatabaseTab } from './components/database/DatabaseTab';
import { StorageTab } from './components/storage/StorageTab';
import { AuthTab } from './components/auth/AuthTab';
import { VersionsTab } from './components/versions/VersionsTab';
import { McpTab } from './components/mcp/McpTab';

function AppContent() {
  const { activeTab } = useDevTools();
  // Initialize theme syncing with Chrome DevTools
  useTheme();

  return (
    <>
      <Tabs />
      <div class="content">
        <div class="tab-content" classList={{ active: activeTab() === 'events' }}>
          <Show when={activeTab() === 'events'}>
            <EventsTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'queries' }}>
          <Show when={activeTab() === 'queries'}>
            <QueriesTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'timing' }}>
          <Show when={activeTab() === 'timing'}>
            <TimingTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'database' }}>
          <Show when={activeTab() === 'database'}>
            <DatabaseTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'storage' }}>
          <Show when={activeTab() === 'storage'}>
            <StorageTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'auth' }}>
          <Show when={activeTab() === 'auth'}>
            <AuthTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'versions' }}>
          <Show when={activeTab() === 'versions'}>
            <VersionsTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'mcp' }}>
          <Show when={activeTab() === 'mcp'}>
            <McpTab />
          </Show>
        </div>
      </div>
    </>
  );
}

export function App() {
  return (
    <DevToolsProvider>
      <AppContent />
    </DevToolsProvider>
  );
}
