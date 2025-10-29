import { createContext, useContext, createSignal, JSX, Show } from "solid-js";
import { db, dbConfig, type Schema } from "../db";
import type { Model } from "@spooky/client-solid";

type User = Model<Schema["user"]>;

interface AuthContextType {
  user: () => User | null;
  isLoading: () => boolean;
  signIn: (username: string, password: string) => Promise<void>;
  signUp: (username: string, password: string) => Promise<void>;
  signOut: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType>();

export function AuthProvider(props: { children: JSX.Element }) {
  const [user, setUser] = createSignal<User | null>(null);
  const [isLoading, setIsLoading] = createSignal(true);

  // Check for existing session on mount
  const checkAuth = async () => {
    const authUser = await db.checkAuth("user");
    if (authUser) {
      setUser(authUser as User);
    }
    setIsLoading(false);
  };

  // Initialize auth check
  checkAuth();

  const signIn = async (username: string, password: string) => {
    try {
      // Use the centralized signIn method from db object
      await db.signIn({
        access: "account",
        variables: {
          username,
          password,
        },
      });
      // Update user from db
      const currentUser = db.getCurrentUser<"user">();
      setUser(currentUser.value as User | null);
    } catch (error) {
      console.error("Sign in failed:", error);
      throw error;
    }
  };

  const signUp = async (username: string, password: string) => {
    try {
      // Use the centralized signUp method from db object
      await db.signUp({
        namespace: dbConfig.namespace,
        database: dbConfig.database,
        access: "account",
        variables: {
          username,
          password,
        },
      });
      // Update user from db
      const currentUser = db.getCurrentUser<"user">();
      setUser(currentUser.value as User | null);
    } catch (error) {
      console.error("Sign up failed:", error);
      throw error;
    }
  };

  const signOut = async () => {
    try {
      await db.signOut();
      // Clear user
      setUser(null);
    } catch (error) {
      console.error("Sign out failed:", error);
      throw error;
    }
  };

  const authValue: AuthContextType = {
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
