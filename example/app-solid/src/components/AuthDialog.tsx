import { createSignal, Show } from "solid-js";
import { useAuth } from "../lib/auth";

interface AuthDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function AuthDialog(props: AuthDialogProps) {
  const auth = useAuth();
  const [isSignUp, setIsSignUp] = createSignal(false);
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setIsLoading(true);

    try {
      if (isSignUp()) {
        await auth.signUp(username(), password());
      } else {
        await auth.signIn(username(), password());
      }
      // Wait a bit for the auth state to fully update
      await new Promise(resolve => setTimeout(resolve, 100));
      
      // Clear form and close dialog
      setUsername("");
      setPassword("");
      props.onClose();
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
      <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
        <div class="bg-white rounded-lg p-6 w-full max-w-md mx-4">
          <div class="flex justify-between items-center mb-4">
            <h2 class="text-xl font-bold">
              {isSignUp() ? "Sign Up" : "Sign In"}
            </h2>
            <button
              onClick={handleClose}
              class="text-gray-500 hover:text-gray-700"
            >
              ✕
            </button>
          </div>

          <form onSubmit={handleSubmit} class="space-y-4">
            <div>
              <label for="username" class="block text-sm font-medium mb-1">
                Username
              </label>
              <input
                id="username"
                type="text"
                value={username()}
                onInput={(e) => setUsername(e.currentTarget.value)}
                required
                class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="Enter username"
              />
            </div>

            <div>
              <label for="password" class="block text-sm font-medium mb-1">
                Password
              </label>
              <input
                id="password"
                type="password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
                required
                class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="Enter password"
              />
            </div>

            <Show when={error()}>
              <div class="text-red-600 text-sm">{error()}</div>
            </Show>

            <button
              type="submit"
              disabled={isLoading()}
              class="w-full bg-blue-600 text-white py-2 px-4 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isLoading() ? "Loading..." : isSignUp() ? "Sign Up" : "Sign In"}
            </button>
          </form>

          <div class="mt-4 text-center">
            <button
              onClick={() => setIsSignUp(!isSignUp())}
              class="text-blue-600 hover:text-blue-800 text-sm"
            >
              {isSignUp()
                ? "Already have an account? Sign In"
                : "Don't have an account? Sign Up"}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
