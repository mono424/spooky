import { createSignal, Show, createEffect } from "solid-js";
import { useAuth } from "../lib/auth";
import { useKeyboard } from "../lib/keyboard";
import { db } from "../db";

interface AuthDialogProps {
  isOpen: boolean;
  onClose: () => void;
  initialMode?: "signin" | "signup";
}

export function AuthDialog(props: AuthDialogProps) {
  const auth = useAuth();
  const [isSignUp, setIsSignUp] = createSignal(false);
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);

  createEffect(() => {
    if (props.isOpen) {
      setIsSignUp(props.initialMode === "signup");
      setError("");
      setUsername("");
      setPassword("");
    }
  });



  useKeyboard({
    "Escape": () => {
        if (props.isOpen) {
            props.onClose();
        }
    }
  });

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setIsLoading(true);

    try {
      if (isSignUp()) {
        // TODO: REMOVE. POLYFLL BECAUSE IT JUST DOES NOT WORK OTHERWISE
        await db.remote.query("fn::polyfill::createAccount($username,$password)", {
          "username": username(),
          "password": password(),
        });
        await auth.signIn(username(), password());
        // TODO: REMOVE. POLYFLL BECAUSE IT JUST DOES NOT WORK OTHERWISE
        // await auth.signUp(username(), password());
      } else {
        await auth.signIn(username(), password());
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
      handleClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "An error occurred");
    } finally {
      setIsLoading(false);
    }
  };

  const handleClose = () => {
    setUsername("");
    setPassword("");
    setError("");
    props.onClose();
  };

  return (
    <Show when={props.isOpen}>
      <style>{`
        /* 1. Animation for the modal */
        @keyframes terminal-boot {
          0% { opacity: 0; transform: scale(0.9) translateY(20px); }
          100% { opacity: 1; transform: scale(1) translateY(0); }
        }
        .animate-terminal {
          animation: terminal-boot 0.2s cubic-bezier(0, 0, 0.2, 1) forwards;
        }

        /* 2. CHROME AUTOFILL FIX */
        /* Forces the background to be black using a shadow, and text to be white */
        input:-webkit-autofill,
        input:-webkit-autofill:hover, 
        input:-webkit-autofill:focus, 
        input:-webkit-autofill:active {
            -webkit-box-shadow: 0 0 0 30px black inset !important;
            -webkit-text-fill-color: white !important;
            caret-color: white;
            transition: background-color 5000s ease-in-out 0s;
        }
      `}</style>

      <div class="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-[100] p-4">
        {/* Main Modal Container */}
        <div class="animate-terminal bg-black border-2 border-white w-full max-w-md relative shadow-[8px_8px_0px_0px_rgba(255,255,255,1)] flex flex-col">
          
          {/* Header */}
          <div class="flex justify-between items-stretch border-b-2 border-white h-12">
            <div class="flex items-center px-4 border-r-2 border-white bg-white text-black font-bold uppercase tracking-widest text-sm">
               [KEY]
            </div>
            <div class="flex-grow flex items-center px-4 font-mono text-sm uppercase tracking-wider">
               {isSignUp() ? "INIT_REGISTRATION" : "AUTH_SEQUENCE"}
            </div>
            <button
              onMouseDown={handleClose}
              class="px-5 hover:bg-white hover:text-black border-l-2 border-white font-bold transition-none text-lg flex items-center justify-center"
              aria-label="Close"
            >
              ✕
            </button>
          </div>

          {/* Content Wrapper */}
          <div class="p-8">
            <form onSubmit={handleSubmit} class="space-y-6">
              
              {/* Username Input */}
              <div class="relative group">
                <label for="username" class="absolute -top-2.5 left-2 bg-black px-2 text-[10px] uppercase font-bold tracking-wider border border-white group-focus-within:border-white z-10">
                  Username
                </label>
                <input
                  id="username"
                  type="text"
                  value={username()}
                  onInput={(e) => setUsername(e.currentTarget.value)}
                  required
                  class="w-full bg-black border-2 border-white px-4 py-3 text-white focus:outline-none focus:shadow-[4px_4px_0px_0px_rgba(255,255,255,1)] transition-none placeholder-gray-700 font-mono text-sm"
                  placeholder="enter_id..."
                  autocomplete="username"
                />
              </div>

              {/* Password Input */}
              <div class="relative group">
                <label for="password" class="absolute -top-2.5 left-2 bg-black px-2 text-[10px] uppercase font-bold tracking-wider border border-white z-10">
                  Password
                </label>
                <input
                  id="password"
                  type="password"
                  value={password()}
                  onInput={(e) => setPassword(e.currentTarget.value)}
                  required
                  class="w-full bg-black border-2 border-white px-4 py-3 text-white focus:outline-none focus:shadow-[4px_4px_0px_0px_rgba(255,255,255,1)] transition-none placeholder-gray-700 font-mono text-sm"
                  placeholder="********"
                  autocomplete={isSignUp() ? "new-password" : "current-password"}
                />
              </div>

              <Show when={error()}>
                <div class="border border-red-500 text-red-500 p-3 text-xs font-mono uppercase">
                  <span class="font-bold">! CRITICAL_ERROR:</span> {error()}
                </div>
              </Show>

              {/* Submit Button */}
              <button
                type="submit"
                disabled={isLoading()}
                class="w-full bg-white text-black border-2 border-white py-3 px-4 uppercase font-bold hover:bg-black hover:text-white hover:border-white transition-none disabled:opacity-50 disabled:cursor-not-allowed text-sm tracking-widest mt-2"
              >
                {isLoading() ? (
                  <span class="animate-pulse">PROCESSING...</span>
                ) : isSignUp() ? (
                  "[ EXECUTE_SIGN_UP ]"
                ) : (
                  "[ EXECUTE_LOGIN ]"
                )}
              </button>
            </form>

            {/* Toggle Link */}
            <div class="mt-8 text-center flex items-center justify-center gap-2 text-xs uppercase text-gray-400">
               <span>&gt;</span>
              <button
                onMouseDown={() => setIsSignUp(!isSignUp())}
                class="hover:text-white hover:underline decoration-white underline-offset-4 transition-none"
              >
                {isSignUp()
                  ? "Access_Existing_Account"
                  : "Create_New_Identifier"}
              </button>
               <span class="animate-blink">_</span>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}