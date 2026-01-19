import { Show } from 'solid-js';
import { DevToolsProvider, useDevTools } from './context/DevToolsContext';
import { useTheme } from './hooks/useTheme';
import { Tabs } from './components/Tabs';
import { EventsTab } from './components/events/EventsTab';
import { QueriesTab } from './components/queries/QueriesTab';
import { DatabaseTab } from './components/database/DatabaseTab';
import { AuthTab } from './components/auth/AuthTab';

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

        <div class="tab-content" classList={{ active: activeTab() === 'database' }}>
          <Show when={activeTab() === 'database'}>
            <DatabaseTab />
          </Show>
        </div>

        <div class="tab-content" classList={{ active: activeTab() === 'auth' }}>
          <Show when={activeTab() === 'auth'}>
            <AuthTab />
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
