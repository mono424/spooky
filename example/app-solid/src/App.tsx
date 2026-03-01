import { Router, Route } from '@solidjs/router';
import { createSignal, Show, createEffect } from 'solid-js';
import { SpookyProvider } from '@spooky-sync/client-solid';
import { AuthProvider, useAuth } from './lib/auth';
import { dbConfig } from './db';
import { AuthDialog } from './components/AuthDialog';
import { PendingMutationsIndicator } from './components/PendingMutationsIndicator';
import { useKeyboard, useShortcutsHelp } from './lib/keyboard';
import { ShortcutsHelp } from './components/ShortcutsHelp';
import { useNavigate } from '@solidjs/router';

// Import routes
import Home from './routes/index';
import ThreadPage from './routes/thread/[id]';
import CreateThreadPage from './routes/create-thread';

function Layout(props: any) {
  const auth = useAuth();
  const navigate = useNavigate();
  const { toggle: toggleHelp } = useShortcutsHelp();
  const [showAuthDialog, setShowAuthDialog] = createSignal(false);
  const [authMode, setAuthMode] = createSignal<'signin' | 'signup'>('signin');

  createEffect(() => {
    if (auth.user()) {
      setShowAuthDialog(false);
    }
  });

  useKeyboard({
    '?': () => toggleHelp(),
    'c': () => navigate('/create-thread'),
    'g h': () => navigate('/'),
  });

  const openAuth = (mode: 'signin' | 'signup') => {
    setAuthMode(mode);
    setShowAuthDialog(true);
  };

  const userInitial = () => {
    const name = auth.user()?.username;
    return name ? name.charAt(0).toUpperCase() : '?';
  };

  return (
    <div class="min-h-screen bg-zinc-950 text-white font-sans selection:bg-accent/30 selection:text-white flex flex-col">
      {/* Header */}
      <header class="bg-zinc-950/80 backdrop-blur-md border-b border-white/[0.06] sticky top-0 z-50 h-14">
        <div class="max-w-5xl mx-auto px-6 h-full">
          <div class="flex justify-between items-center h-full">
            <span class="text-lg font-semibold tracking-tight">threads</span>

            <Show
              when={auth.userId()}
              fallback={
                <div class="flex items-center gap-3">
                  <button
                    onMouseDown={() => openAuth('signin')}
                    class="text-sm text-zinc-400 hover:text-white transition-colors duration-150"
                  >
                    Sign in
                  </button>
                  <button
                    onMouseDown={() => openAuth('signup')}
                    class="bg-accent hover:bg-accent-hover text-white text-sm font-medium px-4 py-1.5 rounded-lg transition-colors duration-150"
                  >
                    Sign up
                  </button>
                </div>
              }
            >
              <div class="flex items-center gap-3">
                <div class="flex items-center gap-2">
                  <div class="w-7 h-7 rounded-full bg-accent/20 text-accent flex items-center justify-center text-xs font-semibold">
                    {userInitial()}
                  </div>
                  <span class="text-sm text-zinc-300 hidden sm:inline">
                    {auth.user()?.username}
                  </span>
                </div>
                <button
                  onMouseDown={auth.signOut}
                  class="text-sm text-zinc-500 hover:text-white transition-colors duration-150"
                >
                  Sign out
                </button>
              </div>
            </Show>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main class="flex-grow">
        <Show
          when={auth.userId()}
          fallback={
            <div class="h-full flex items-center justify-center min-h-[80vh] px-6">
              <div class="text-center w-full max-w-sm">
                {/* Logo mark */}
                <div class="w-14 h-14 rounded-2xl bg-accent/10 border border-accent/20 flex items-center justify-center mx-auto mb-6">
                  <svg class="w-7 h-7 text-accent" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M7.5 8.25h9m-9 3H12m-9.75 1.51c0 1.6 1.123 2.994 2.707 3.227 1.129.166 2.27.293 3.423.379.35.026.67.21.865.501L12 21l2.755-4.133a1.14 1.14 0 01.865-.501 48.172 48.172 0 003.423-.379c1.584-.233 2.707-1.626 2.707-3.228V6.741c0-1.602-1.123-2.995-2.707-3.228A48.394 48.394 0 0012 3c-2.392 0-4.744.175-7.043.513C3.373 3.746 2.25 5.14 2.25 6.741v6.018z" />
                  </svg>
                </div>

                <h2 class="text-3xl font-semibold mb-2 tracking-tight">
                  Welcome to Threads
                </h2>
                <p class="text-zinc-400 text-sm mb-8 max-w-xs mx-auto">
                  A modern place for conversations. Sign in to join the discussion.
                </p>

                <div class="flex flex-col gap-3">
                  <button
                    onMouseDown={() => openAuth('signin')}
                    class="w-full bg-accent hover:bg-accent-hover text-white px-6 py-3 rounded-lg font-medium transition-colors duration-150"
                  >
                    Sign in
                  </button>
                  <button
                    onMouseDown={() => openAuth('signup')}
                    class="w-full bg-surface hover:bg-surface-hover text-zinc-300 border border-white/[0.06] px-6 py-3 rounded-lg font-medium transition-colors duration-150"
                  >
                    Create account
                  </button>
                </div>
              </div>
            </div>
          }
        >
          <div class="max-w-5xl mx-auto px-6 py-4 min-h-screen">
            {props.children}
          </div>
        </Show>
      </main>

      <AuthDialog
        isOpen={showAuthDialog()}
        onClose={() => setShowAuthDialog(false)}
        initialMode={authMode()}
      />

      <PendingMutationsIndicator />
      <ShortcutsHelp />
    </div>
  );
}

export default function App() {
  return (
    <SpookyProvider
      config={dbConfig}
      fallback={
        <div class="min-h-screen bg-zinc-950 text-white font-sans flex flex-col items-center justify-center gap-3">
          <svg class="animate-spin h-5 w-5 text-accent" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
          </svg>
          <span class="text-sm text-zinc-400">Connecting...</span>
        </div>
      }
      onError={(error) => console.error('Failed to initialize database:', error)}
    >
      <AuthProvider>
        <Router root={Layout}>
          <Route path="/" component={Home} />
          <Route path="/thread/:id" component={ThreadPage} />
          <Route path="/create-thread" component={CreateThreadPage} />
        </Router>
      </AuthProvider>
    </SpookyProvider>
  );
}
