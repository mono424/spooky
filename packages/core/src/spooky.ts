import { QueryManager } from './services/query/query.js';
import { MutationManager } from './services/mutation/mutation.js';
import { SpookyConfig, QueryTimeToLive } from './types.js';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index.js';
import { SpookySync } from './services/sync/index.js';
import { SchemaStructure } from '@spooky/query-builder';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private migrator: LocalMigrator;
  private queryManager: QueryManager;
  private mutationManager: MutationManager;
  private sync: SpookySync;

  constructor(private config: SpookyConfig<S>) {
    if (!this.config.clientId) {
      this.config.clientId = crypto.randomUUID();
    }
    this.local = new LocalDatabaseService(this.config.database);
    this.remote = new RemoteDatabaseService(this.config.database);
    this.migrator = new LocalMigrator(this.local);
    this.mutationManager = new MutationManager(this.local);
    this.queryManager = new QueryManager(this.local, this.remote, this.config.clientId);
    this.sync = new SpookySync(
      this.local,
      this.remote,
      this.mutationManager.events,
      this.queryManager.eventsSystem
    );
  }

  async init() {
    await this.local.connect();
    await this.remote.connect();
    await this.migrator.provision(this.config.schemaSurql);
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

  async query(sql: string, params: Record<string, any>, ttl: QueryTimeToLive) {
    return this.queryManager.query(sql, params, ttl);
  }

  async subscribe(queryHash: string, callback: () => void) {
    return this.queryManager.subscribe(queryHash, callback);
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
