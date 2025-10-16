import { Router, Routes, Route } from "solid-router";
import { createSignal, Show, onMount } from "solid-js";
import { AuthProvider, useAuth } from "./lib/auth";
import { initDatabase } from "./lib/db";
import { AuthDialog } from "./components/AuthDialog";

// Import routes
import Home from "./routes/index";
import ThreadPage from "./routes/thread/[id]";
import CreateThreadPage from "./routes/create-thread";

function AppContent() {
  const auth = useAuth();
  const [showAuthDialog, setShowAuthDialog] = createSignal(false);

  // Show auth dialog if user is not authenticated
  const handleAuthRequired = () => {
    setShowAuthDialog(true);
  };

  return (
    <div class="min-h-screen bg-gray-50">
      {/* Header */}
      <header class="bg-white shadow-sm border-b">
        <div class="max-w-4xl mx-auto px-4 py-3">
          <div class="flex justify-between items-center">
            <h1 class="text-xl font-bold text-gray-900">Thread App</h1>
            <Show
              when={auth.user()}
              fallback={
                <button
                  onClick={handleAuthRequired}
                  class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700"
                >
                  Sign In
                </button>
              }
            >
              <div class="flex items-center space-x-4">
                <span class="text-gray-700">
                  Welcome, {auth.user()!.username}
                </span>
                <button
                  onClick={auth.signOut}
                  class="text-gray-600 hover:text-gray-800"
                >
                  Sign Out
                </button>
              </div>
            </Show>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main>
        <Show
          when={auth.user()}
          fallback={
            <div class="max-w-4xl mx-auto p-4">
              <div class="text-center py-12">
                <h2 class="text-2xl font-bold mb-4">Welcome to Thread App</h2>
                <p class="text-gray-600 mb-6">
                  Sign in to view and create threads
                </p>
                <button
                  onClick={handleAuthRequired}
                  class="bg-blue-600 text-white px-6 py-3 rounded-md hover:bg-blue-700"
                >
                  Get Started
                </button>
              </div>
            </div>
          }
        >
          <Routes>
            <Route path="/" component={Home} />
            <Route path="/thread/:id" component={ThreadPage} />
            <Route path="/create-thread" component={CreateThreadPage} />
          </Routes>
        </Show>
      </main>

      {/* Auth Dialog */}
      <AuthDialog
        isOpen={showAuthDialog()}
        onClose={() => setShowAuthDialog(false)}
      />
    </div>
  );
}

export default function App() {
  const [isDbReady, setIsDbReady] = createSignal(false);

  // Initialize database on app start
  onMount(async () => {
    try {
      await initDatabase();
      setIsDbReady(true);
    } catch (error) {
      console.error("Failed to initialize database:", error);
      // Still show the app, but database operations will fail
      setIsDbReady(true);
    }
  });

  return (
    <Show
      when={isDbReady()}
      fallback={
        <div class="min-h-screen flex items-center justify-center">
          <div class="text-lg">Initializing...</div>
        </div>
      }
    >
      <AuthProvider>
        <Router>
          <AppContent />
        </Router>
      </AuthProvider>
    </Show>
  );
}
