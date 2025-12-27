import {
  createContext,
  useContext,
  createSignal,
  JSX,
  Show,
  onCleanup,
} from "solid-js";
import { db, dbConfig } from "../db";
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

  const user = () => userQuery.data() || null;

  // Check for existing session on mount
  const checkAuth = async (tkn?: string) => {
    const token = tkn || localStorage.getItem("token");
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
      const users = await db.useRemote(async (remote) => {
         return await remote.query('SELECT * FROM user WHERE id = $auth.id');
      });

      // users is ActionResult[]
      const firstRes = Array.isArray(users) ? users[0] : users;
      const records = (firstRes as any).result || firstRes;
      const currentUser = Array.isArray(records) && records.length > 0 ? records[0] : null;

      if (currentUser && currentUser.id) {
        setUserId(currentUser.id.toString());
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
      const res = await db.useRemote((db) =>
        db.signin({
          access: "account", // namespace 'main' / database 'main' / access 'account'
          variables: {
            username,
            password,
          },
        })
      );

      console.log("[AuthProvider] Sign in successful, token received");
      // res is the token (string) in v2 usually, or { token: string }?
      // In v2 it returns string (the token).
      await checkAuth(res as unknown as string);
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
      const res = await db.useRemote((db) =>
        db.signup({
          access: "account",
          variables: {
            username,
            password,
          },
        })
      );

      console.log("[AuthProvider] Sign up successful, token received");
      await checkAuth(res as unknown as string);
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
