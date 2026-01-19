import { createContext, useContext, createSignal, JSX, Show, onCleanup } from 'solid-js';
import { db } from '../db';
import { schema } from '../schema.gen';
import { type GetTable, type TableModel, useQuery } from '@spooky/client-solid';

type User = TableModel<GetTable<typeof schema, 'user'>>;

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
  const [isInitializing, setIsInitializing] = createSignal(true);

  // Subscribe to auth state
  const unsubscribe = db.auth.subscribe((uid) => {
    setUserId(uid);
    setIsInitializing(false);
  });

  onCleanup(() => unsubscribe());

  // Only run query after auth is initialized and userId is available
  const userQuery = useQuery(
    db,
    () => {
      const currentUserId = userId();
      if (!currentUserId) {
        return null;
      }
      return db.query('user').where({ id: currentUserId }).one().build();
    },
    {
      enabled: () => userId() !== null,
    }
  );

  const user = () => {
    const data = userQuery.data();
    return data || null;
  };

  const signIn = async (username: string, password: string) => {
    await db.auth.signIn('account', { username, password });
  };

  const signUp = async (username: string, password: string) => {
    await db.auth.signUp('account', { username, password });
  };

  const signOut = async () => {
    await db.auth.signOut();
  };

  const authValue: AuthContextType = {
    userId,
    user,
    isLoading: () => isInitializing() || (!!userId() && userQuery.isLoading()),
    signIn,
    signUp,
    signOut,
  };

  return (
    <AuthContext.Provider value={authValue}>
      <Show
        when={!authValue.isLoading()}
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
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
