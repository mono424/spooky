import { QueryManager } from './services/query/query.js';
import { MutationManager } from './services/mutation/mutation.js';
import { SpookyConfig, QueryTimeToLive, SpookyQueryResultPromise } from './types.js';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index.js';
import { Surreal } from 'surrealdb';
import { SpookySync } from './services/sync/index.js';
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from '@spooky/query-builder';

import { DevToolsService } from './services/devtools-service.js';
import { createLogger } from './services/logger.js';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private migrator: LocalMigrator;
  private queryManager: QueryManager<S>;
  private mutationManager: MutationManager<S>;
  private sync: SpookySync<S>;
  private devTools: DevToolsService;

  get remoteClient() {
    return this.remote.getClient();
  }

  get localClient() {
    return this.local.getClient();
  }

  constructor(private config: SpookyConfig<S>) {
    const clientId = this.config.clientId ?? this.loadOrGenerateClientId();
    this.persistClientId(clientId);

    const logger = createLogger('info'); // Default logger
    this.local = new LocalDatabaseService(this.config.database);
    this.remote = new RemoteDatabaseService(this.config.database);
    this.migrator = new LocalMigrator(this.local);
    this.mutationManager = new MutationManager(this.config.schema, this.local);
    this.queryManager = new QueryManager(this.config.schema, this.local, this.remote, clientId);
    this.sync = new SpookySync(
      this.config.schema,
      this.local,
      this.remote,
      this.mutationManager.events,
      this.queryManager.eventsSystem,
      clientId
    );
    this.devTools = new DevToolsService(
      this.mutationManager.events,
      this.queryManager.eventsSystem,
      this.local,
      logger,
      this.config.schema
    );
  }

  async init() {
    await this.local.connect();
    await this.remote.connect();
    await this.queryManager.init();
    await this.migrator.provision(this.config.schemaSurql);
    await this.sync.init();
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

  query<Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>,
    ttl: QueryTimeToLive = '10m'
  ): QueryBuilder<S, Table, SpookyQueryResultPromise> {
    return new QueryBuilder<S, Table, SpookyQueryResultPromise>(
      this.config.schema,
      table,
      async (q) => ({
        hash: await this.queryManager.query(
          table,
          q.selectQuery.query,
          q.selectQuery.vars ?? {},
          ttl
        ),
      }),
      options
    );
  }

  async queryRaw(sql: string, params: Record<string, any>, ttl: QueryTimeToLive) {
    const tableName = sql.split('FROM ')[1].split(' ')[0];
    return this.queryManager.query(tableName, sql, params, ttl);
  }

  async subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options?: { immediate?: boolean }
  ): Promise<() => void> {
    return this.queryManager.subscribe(queryHash, callback, options);
  }

  create(id: string, data: Record<string, unknown>) {
    return this.mutationManager.create(id, data);
  }

  update(table: string, id: string, data: Record<string, unknown>) {
    return this.mutationManager.update(table, id, data);
  }

  delete(table: string, id: string) {
    return this.mutationManager.delete(table, id);
  }

  async useRemote<T>(fn: (client: Surreal) => Promise<T> | T): Promise<T> {
    return fn(this.remote.getClient());
  }

  private persistClientId(id: string) {
    try {
      if (typeof localStorage !== 'undefined') {
        localStorage.setItem('spooky_client_id', id);
      }
    } catch (e) {
      console.warn('[SpookyClient] Failed to persist client ID', e);
    }
  }

  private loadOrGenerateClientId(): string {
    try {
      if (typeof localStorage !== 'undefined') {
        const stored = localStorage.getItem('spooky_client_id');
        if (stored) return stored;
      }
    } catch (e) {
      console.warn('[SpookyClient] Failed to load client ID', e);
    }

    const newId = crypto.randomUUID();
    this.persistClientId(newId);
    return newId;
  }
}
