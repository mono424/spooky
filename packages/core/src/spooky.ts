import { DatabaseService } from "./services/database.js";
import { AuthManager } from "./services/auth.js";
import { QueryManager } from "./services/query.js";
import { MutationManager } from "./services/mutation.js";
import { SpookyConfig } from "./types.js";

export interface SpookyClient {
  query: (surrealql: string) => {
    subscribe: (callback: (data: any) => void) => () => void;
  };
  mutation: {
    create: <T extends Record<string, unknown>>(table: string, data: T) => Promise<T>;
    update: <T extends Record<string, unknown>>(table: string, id: string, data: Partial<T>) => Promise<T>;
    delete: (table: string, id: string) => Promise<void>;
  };
  auth: {
    authenticate: (token: string) => Promise<void>;
    deauthenticate: () => Promise<void>;
  };
  close: () => Promise<void>;
}

export async function createSpooky(config: SpookyConfig): Promise<SpookyClient> {
  const db = new DatabaseService(config);
  await db.init();

  const auth = new AuthManager(db);
  const queryManager = new QueryManager(db);
  const mutationManager = new MutationManager(db);

  return {
    query: (surrealql: string) => {
      // Register query immediately
      const hashPromise = queryManager.register(surrealql);
      
      return {
        subscribe: (callback: (data: any) => void) => {
          let unsubscribe: (() => void) | undefined;
          
          hashPromise.then((hash) => {
            unsubscribe = queryManager.subscribe(hash, callback);
          });

          return () => {
            if (unsubscribe) unsubscribe();
          };
        },
      };
    },
    mutation: {
      create: <T extends Record<string, unknown>>(table: string, data: T) => mutationManager.create<T>(table, data),
      update: <T extends Record<string, unknown>>(table: string, id: string, data: Partial<T>) => mutationManager.update<T>(table, id, data),
      delete: (table, id) => mutationManager.delete(table, id),
    },
    auth: {
      authenticate: (token) => auth.authenticate(token),
      deauthenticate: () => auth.deauthenticate(),
    },
    close: async () => {
      await db.close();
    },
  };
}
