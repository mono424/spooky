import { createContext, useContext, createSignal, JSX, Show } from "solid-js";
import { db, dbConfig } from "../db";
import { schema } from "../schema.gen";
import { type GetTable, type TableModel } from "@spooky/client-solid";
import { useQuery } from "@tanstack/solid-query";

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
  const [userId, setUserId] = createSignal<string | null>(null);
  const [isLoading, setIsLoading] = createSignal(true);
  const [isInitialized, setIsInitialized] = createSignal(false);

  // Only run query after auth is initialized
  const userQuery = useQuery(() => ({
    queryKey: ["user"],
    queryFn: async () => {
      if (!isInitialized()) return null;
      try {
        return await db.query("user").one().build();
      } catch (error) {
        console.error("Failed to fetch user:", error);
        return null;
      }
    },
    enabled: isInitialized(),
  }));

  const user = () => {
    return userQuery.data ?? null;
  };

  // Check for existing session on mount
  const checkAuth = async (tkn?: string) => {
    const token = tkn || localStorage.getItem("token");
    console.log("[AuthProvider] Checking auth with token:", !!token);
    if (token) {
      try {
        const userId = await db.authenticate(token);
        if (userId) {
          setUserId(userId.id.toString());
          localStorage.setItem("token", token);
          console.log(
            "[AuthProvider] Auth check successful, userId:",
            userId.id
          );
        }
      } catch (error) {
        console.error("[AuthProvider] Auth check failed:", error);
        localStorage.removeItem("token");
      }
    }
    setIsInitialized(true);
    setIsLoading(false);
  };

  // Initialize auth check
  checkAuth();

  const signIn = async (username: string, password: string) => {
    try {
      // Use the centralized signIn method from db object
      const res = await db.db().signin({
        access: "account",
        variables: {
          username,
          password,
        },
      });

      await checkAuth(res.token);
    } catch (error) {
      console.error("Sign in failed:", error);
      throw error;
    }
  };

  const signUp = async (username: string, password: string) => {
    try {
      // Use the centralized signUp method from db object
      const res = await db.db().signup({
        namespace: dbConfig.namespace,
        database: dbConfig.database,
        access: "account",
        variables: {
          username,
          password,
        },
      });

      await checkAuth(res.token);
    } catch (error) {
      console.error("Sign up failed:", error);
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
