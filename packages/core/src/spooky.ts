import { QueryManager } from "./services/query.js";
import { MutationManager } from "./services/mutation/mutation.js";
import { SpookyConfig } from "./types.js";
import { LocalDatabaseService, RemoteDatabaseService } from "./services/database/index.js";

export class SpookyClient {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private queryManager: QueryManager;
  private mutationManager: MutationManager;

  constructor(config: SpookyConfig) {
    this.local = new LocalDatabaseService(config);
    this.remote = new RemoteDatabaseService(config);
    this.mutationManager = new MutationManager(this.local);
    this.queryManager = new QueryManager(this.local);
  }

  async init() {
    await this.local.connect();
    await this.remote.connect();
  }

  async close() {
    await this.local.close();
    await this.remote.close();
  }

  authenticate(token: string) {
    return this.remote.getClient().authenticate(token);
  }

  deauthenticate() {
    return this.remote.getClient().invalidate();
  }

  query(surrealql: string) {
    const hashPromise = this.queryManager.register(surrealql);
    
    return {
      subscribe: (callback: (data: any) => void) => {
        let unsubscribe: (() => void) | undefined;
        
        hashPromise.then((hash) => {
          unsubscribe = this.queryManager.subscribe(hash, callback);
          });

          return () => {
            if (unsubscribe) unsubscribe();
          };
        },
      };
  }

  create(table: string, data: Record<string, unknown>) {
    return this.mutationManager.create(table, data);
  }

  update(table: string, id: string, data: Record<string, unknown>) {
    return this.mutationManager.update(table, id, data);
  }

  delete(table: string, id: string) {
    return this.mutationManager.delete(table, id);
  }
}
