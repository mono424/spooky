import { createContext, useContext, createSignal, JSX, Show } from "solid-js";
import { db, dbConfig, type Schema } from "../db";
import type { Model } from "db-solid";

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
    console.log("[auth] Checking authentication");
    try {
      const token = localStorage.getItem("auth_token");
      if (token) {
        // Authenticate with stored token
        console.log("[auth] Authenticating with token");
        await db.authenticate(token);

        console.log("[auth] Querying authenticated user info");
        // Query authenticated user info
        const [users] = await db.query.user
          .queryLocal(`SELECT * FROM $auth`)
          .collect();

        console.log("[auth] Authenticated user info", users);
        if (users && users.length > 0) {
          setUser(users[0]);
        } else {
          localStorage.removeItem("auth_token");
        }
      }
    } catch (error) {
      console.error("Auth check failed:", error);
      localStorage.removeItem("auth_token");
    } finally {
      setIsLoading(false);
    }
  };

  // Initialize auth check
  checkAuth();

  const signIn = async (username: string, password: string) => {
    try {
      const localDb = db.getLocal();

      const authResponse = await localDb.signin({
        access: "account",
        variables: {
          username,
          password,
        },
      });

      if (authResponse.token) {
        localStorage.setItem("auth_token", authResponse.token);

        // Query authenticated user info
        const [users] = await db.query.user
          .queryLocal(`SELECT * FROM $auth`)
          .collect();

        if (users && users.length > 0) {
          setUser(users[0]);
        }
      }
    } catch (error) {
      console.error("Sign in failed:", error);
      throw error;
    }
  };

  const signUp = async (username: string, password: string) => {
    try {
      const localDb = db.getLocal();
      const { namespace, database } = dbConfig;

      // Use SurrealDB's signup method
      const authResponse = await localDb.signup({
        namespace,
        database,
        access: "account",
        variables: {
          username,
          password,
        },
      });

      if (authResponse.token) {
        localStorage.setItem("auth_token", authResponse.token);

        // Query authenticated user info
        const [users] = await db.query.user
          .queryLocal(`SELECT * FROM $auth`)
          .collect();

        if (users && users.length > 0) {
          setUser(users[0]);
        }
      }
    } catch (error) {
      console.error("Sign up failed:", error);
      throw error;
    }
  };

  const signOut = async () => {
    try {
      const localDb = db.getLocal();
      await localDb.invalidate();
    } catch (error) {
      console.error("Sign out failed:", error);
    } finally {
      setUser(null);
      localStorage.removeItem("auth_token");
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
