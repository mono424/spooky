import { createContext, useContext, createSignal, JSX, Show } from "solid-js";
import { db } from "./db";
import { Model, TempSchema } from "db-solid";

type User = Model<TempSchema["user"]>;

interface AuthContextType {
  user: () => User | null;
  isLoading: () => boolean;
  signIn: (username: string, password: string) => Promise<void>;
  signUp: (username: string, password: string) => Promise<void>;
  signOut: () => void;
}

const AuthContext = createContext<AuthContextType>();

export function AuthProvider(props: { children: JSX.Element }) {
  const [user, setUser] = createSignal<User | null>(null);
  const [isLoading, setIsLoading] = createSignal(true);

  // Check for existing session on mount
  const checkAuth = async () => {
    try {
      const token = localStorage.getItem("auth_token");
      if (token) {
        // Validate token by querying user data
        const [users] = await db.query.user
          .queryLocal(
            `
          SELECT * FROM user WHERE id = $token.sub
        `,
            { token: { sub: token } }
          )
          .collect();

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
      const [users] = await db.query.user
        .queryLocal(
          `
        SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password)
      `,
          { username, password }
        )
        .collect();

      if (users && users.length > 0) {
        const user = users[0];
        setUser(user);
        localStorage.setItem("auth_token", user.id.toJSON());
      } else {
        throw new Error("Invalid username or password");
      }
    } catch (error) {
      console.error("Sign in failed:", error);
      throw error;
    }
  };

  const signUp = async (username: string, password: string) => {
    try {
      const userId = `user:${Date.now()}_${Math.random()
        .toString(36)
        .substring(2, 9)}`;

      const [users] = await db.query.user
        .queryLocal(
          `
        CREATE user SET
          id = $id,
          username = $username,
          password = crypto::argon2::generate($password),
          created_at = time::now()
      `,
          { id: userId, username, password }
        )
        .collect();

      if (users && users.length > 0) {
        const user = users[0];
        setUser(user);
        localStorage.setItem("auth_token", user.id.toJSON());
      } else {
        throw new Error("Failed to create user");
      }
    } catch (error) {
      console.error("Sign up failed:", error);
      throw error;
    }
  };

  const signOut = () => {
    setUser(null);
    localStorage.removeItem("auth_token");
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
