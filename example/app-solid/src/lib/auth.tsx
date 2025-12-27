import {
  createContext,
  useContext,
  createSignal,
  JSX,
  Show,
  onCleanup,
} from "solid-js";
import { db } from "../db";
import { schema } from "../schema.gen";
import {
  type GetTable,
  type TableModel,
  useQuery,
} from "@spooky/client-solid";

type User = TableModel<GetTable<typeof schema, "user">>;

interface AuthContextType {
  userId: () => string | null;
  user: () => User | null;
  isLoading: () => boolean;
  signIn: (username: string, password: string) => Promise<void>;
  signUp: (username: string, password: string) => Promise<void>;
  signOut: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType>();

export function AuthProvider(props: { children: JSX.Element }) {
  const spooky = db.getSpooky();
  const [userId, setUserId] = createSignal<string | null>(null);
  const [remoteUser, setRemoteUser] = createSignal<User | null>(null);
  const [isLoading, setIsLoading] = createSignal(true);
  const [isInitialized, setIsInitialized] = createSignal(false);

  // Remove subIds and onCleanup for events
  
  // Only run query after auth is initialized and userId is available
  const userQuery = useQuery(
    db,
    () => {
      const currentUserId = userId();
      if (!currentUserId) {
        return null;
      }
      return db.query("user").where({ id: currentUserId }).one().build();
    },
    {
      enabled: () => isInitialized() && userId() !== null,
    }
  );

  const user = () => userQuery.data() || remoteUser() || null;

  // Check for existing session on mount
  const checkAuth = async (access?: string) => {
    const token = access || localStorage.getItem("token");
    console.log("[AuthProvider] Checking auth with token:", !!token);
    if (!token) {
      setIsLoading(false);
      setIsInitialized(true);
      return;
    }
    setIsLoading(true);
    try {
      await db.authenticate(token);
      // Fetch current user ID
      // We use useRemote to execute a raw query to get the authenticated user
      const [user] = await db.remote.query('SELECT * FROM ONLY user WHERE id = $auth.id').collect<[User]>();
      console.log("[AuthProvider] Auth check complete, user:", user);

      if (user && user.id) {
        setUserId(user.id.toString());
        setRemoteUser(user);
        localStorage.setItem("token", token);
      } else {
         // Token invalid or user not found
         await signOut(); // Cleanup
      }

    } catch (error) {
       console.error("Auth check failed", error);
       await signOut();
    }
    setIsLoading(false);
    setIsInitialized(true);
  };

  // Initialize auth check
  checkAuth();

  const signIn = async (username: string, password: string) => {
    try {
      console.log("[AuthProvider] Sign in attempt for:", username);
      const { access } = await db.remote.signin({
          access: "account", // namespace 'main' / database 'main' / access 'account'
          variables: {
            username,
            password,
          },
        });
      console.log("[AuthProvider] Sign in result:", access);
      console.log("[AuthProvider] Sign in successful, token received");
      await checkAuth(access);
      console.log("[AuthProvider] Auth check complete after sign in");
    } catch (error) {
      console.error("[AuthProvider] Sign in failed:", error);
      if (error instanceof Error && "cause" in error) {
        console.error("Caused by:", (error as any).cause);
      }
      throw error;
    }
  };

  const signUp = async (username: string, password: string) => {
    try {
      console.log("[AuthProvider] Sign up attempt for:", username);
      const res = await db.remote.query("fn::polyfill::createAccount($username, $password)", {
          username,
          password,
        });

      console.log("[AuthProvider] Sign up successful, attempting sign in...");
      // Auto-login after signup
      await signIn(username, password);
      console.log("[AuthProvider] Auth check complete after sign up");
    } catch (error) {
      console.error("[AuthProvider] Sign up failed:", error);
      if (error instanceof Error && "cause" in error) {
        console.error("Caused by:", (error as any).cause);
      }
      throw error;
    }
  };

  const signOut = async () => {
    try {
      localStorage.removeItem("token");
      setUserId(null);
      setRemoteUser(null);
      await db.deauthenticate();
    } catch (error) {
      console.error("Sign out failed:", error);
      if (error instanceof Error && "cause" in error) {
        console.error("Caused by:", (error as any).cause);
      }
      throw error;
    }
  };

  const authValue: AuthContextType = {
    userId,
    user,
    isLoading,
    signIn,
    signUp,
    signOut,
  };

  return (
    <AuthContext.Provider value={authValue}>
      <Show
        when={!isLoading()}
        fallback={
          <div class="min-h-screen flex items-center justify-center">
            <div class="text-lg">Loading...</div>
          </div>
        }
      >
        {props.children}
      </Show>
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
}
