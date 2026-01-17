import { Router, Route } from "@solidjs/router";
import { createSignal, Show, onMount, createEffect } from "solid-js";
import { AuthProvider, useAuth } from "./lib/auth";
import { initDatabase } from "./db";
import { AuthDialog } from "./components/AuthDialog";
import { useKeyboard, useShortcutsHelp } from "./lib/keyboard";
import { ShortcutsHelp } from "./components/ShortcutsHelp";
import { useNavigate } from "@solidjs/router";

// Import routes
import Home from "./routes/index";
import ThreadPage from "./routes/thread/[id]";
import CreateThreadPage from "./routes/create-thread";

// Defined outside component to preserve whitespace exactly
const THREADS_ASCII = `
████████╗██╗  ██╗██████╗ ███████╗ █████╗ ██████╗ ███████╗
╚══██╔══╝██║  ██║██╔══██╗██╔════╝██╔══██╗██╔══██╗██╔════╝
   ██║   ███████║██████╔╝█████╗  ███████║██║  ██║███████╗
   ██║   ██╔══██║██╔══██╗██╔══╝  ██╔══██║██║  ██║╚════██║
   ██║   ██║  ██║██║  ██║███████╗██║  ██║██████╔╝███████║
   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═════╝ ╚══════╝
`;

function Layout(props: any) {
  const auth = useAuth();
  const [showAuthDialog, setShowAuthDialog] = createSignal(false);
  // Add state to track which mode to open the dialog in
  const [authMode, setAuthMode] = createSignal<"signin" | "signup">("signin");

  createEffect(() => {
    if (auth.user()) {
      setShowAuthDialog(false);
    }
  });

  // Helper to open specific mode
  const openAuth = (mode: "signin" | "signup") => {
    setAuthMode(mode);
    setShowAuthDialog(true);
  };

  return (
    <div class="min-h-screen bg-black text-white font-mono selection:bg-white selection:text-black flex flex-col">
      {/* Header */}
      <header class="bg-black border-b-2 border-white sticky top-0 z-50">
        <div class="max-w-4xl mx-auto px-4 py-4">
          <div class="flex justify-between items-center">
            <div class="flex items-center gap-2">
              <span class="animate-pulse text-white font-bold">&gt;</span>
              <h1 class="text-xl font-bold tracking-widest uppercase">
                [ THREAD_APP ]
              </h1>
            </div>

            <Show
              when={auth.userId()}
              fallback={
                <div class="flex space-x-4">
                   {/* Header Login Button */}
                  <button
                    onMouseDown={() => openAuth("signin")}
                    class="hidden sm:block text-sm uppercase font-bold hover:underline decoration-white underline-offset-4"
                  >
                    Login
                  </button>
                   {/* Header Register Button */}
                  <button
                    onMouseDown={() => openAuth("signup")}
                    class="border-2 border-white px-4 py-1 uppercase font-bold text-sm hover:bg-white hover:text-black transition-none"
                  >
                    [ REGISTER ]
                  </button>
                </div>
              }
            >
              <div class="flex items-center space-x-6 text-sm">
                <span class="text-gray-400 uppercase hidden sm:inline">
                  USER: <span class="text-white">{auth.user()?.username ?? "GUEST"}</span>
                </span>
                <button
                  onMouseDown={auth.signOut}
                  class="hover:underline decoration-white underline-offset-4 uppercase"
                >
                  &lt;&lt; LOGOUT
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
            <div class="max-w-4xl mx-auto p-4 h-full flex items-center justify-center min-h-[80vh]">
              <div class="text-center w-full">
                
                {/* ASCII BOOK LOGO */}
                <div class="flex justify-center mb-10">
                    <pre class="text-[8px] sm:text-[10px] md:text-xs leading-none font-bold whitespace-pre text-white tracking-tight text-center">
                        {THREADS_ASCII}
                    </pre>
                </div>

                <div class="border-2 border-white p-8 max-w-lg mx-auto relative">
                  <div class="absolute -top-3 left-4 bg-black px-2 uppercase font-bold text-sm border-x border-white">
                    System Message
                  </div>
                  
                  <h2 class="text-2xl font-bold mb-6 uppercase tracking-widest">
                    Welcome to Thread App
                  </h2>
                  
                  <p class="text-gray-400 mb-8 border-l-2 border-white pl-4 text-left font-mono text-sm">
                    &gt; Access restricted.<br/>
                    &gt; Authentication required.<br/>
                    &gt; Please initialize session to continue.
                  </p>
                  
                  <div class="flex flex-col gap-4">
                    {/* Hero Login Button */}
                    <button
                        onMouseDown={() => openAuth("signin")}
                        class="w-full border-2 border-white bg-white text-black px-6 py-4 uppercase font-bold hover:bg-black hover:text-white hover:border-white transition-none"
                    >
                        [ INITIALIZE_SESSION ]
                    </button>
                  </div>
                </div>
              </div>
            </div>
          }
        >
          <div class="max-w-4xl mx-auto p-4 border-x border-dashed border-white/20 min-h-screen">
             {props.children}
          </div>
        </Show>
      </main>

      <footer class="border-t border-white py-2 px-4 text-xs uppercase text-gray-500 flex justify-between bg-black">
        <span>Status: ONLINE</span>
        <span>v1.0.0_BUILD_FINAL</span>
      </footer>

      <AuthDialog
        isOpen={showAuthDialog()}
        onClose={() => setShowAuthDialog(false)}
        initialMode={authMode()} // Pass the mode here
      />
    </div>
  );
}

export default function App() {
  const [isDbReady, setIsDbReady] = createSignal(false);

  onMount(async () => {
    try {
      await initDatabase();
      setIsDbReady(true);
    } catch (error) {
      console.error("Failed to initialize database:", error);
      setIsDbReady(true);
    }
  });

  return (
    <Show
      when={isDbReady()}
      fallback={
        <div class="min-h-screen bg-black text-white font-mono flex flex-col items-center justify-center">
          <pre class="mb-4 text-xs animate-pulse">
            [ SYSTEM BOOT ]
          </pre>
          <div class="text-xl font-bold uppercase tracking-widest">
             &gt; Initializing Database...<span class="animate-pulse">_</span>
          </div>
        </div>
      }
    >
      <AuthProvider>
        <Router root={Layout}>
          <Route path="/" component={Home} />
          <Route path="/thread/:id" component={ThreadPage} />
          <Route path="/create-thread" component={CreateThreadPage} />
        </Router>
      </AuthProvider>
    </Show>
  );
}