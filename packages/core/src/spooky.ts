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

import { DevToolsService } from './services/devtools-service/index.js';
import { createLogger } from './services/logger/index.js';
import { AuthService } from './services/auth/index.js';
import { RouterService } from './services/router/index.js';
import { StreamProcessorService } from './services/stream-processor/index.js';
import { EventSystem } from './events/index.js';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private migrator: LocalMigrator;
  private queryManager: QueryManager<S>;
  private mutationManager: MutationManager<S>;
  private sync: SpookySync<S>;
  private devTools: DevToolsService;
  private router: RouterService<S>;
  public auth: AuthService<S>;
  public streamProcessor: StreamProcessorService;

  get remoteClient() {
    return this.remote.getClient();
  }

  get localClient() {
    return this.local.getClient();
  }

  constructor(private config: SpookyConfig<S>) {
    console.log('[Spooky] Constructor called', config);
    const clientId = this.config.clientId ?? this.loadOrGenerateClientId();
    this.persistClientId(clientId);

    const logger = createLogger(config.logLevel ?? 'info');
    this.local = new LocalDatabaseService(this.config.database, logger);
    this.remote = new RemoteDatabaseService(this.config.database, logger);
    this.streamProcessor = new StreamProcessorService(
      new EventSystem(['stream_update']),
      this.local,
      logger
    );
    this.migrator = new LocalMigrator(this.local, logger);
    this.mutationManager = new MutationManager(this.config.schema, this.local, logger);
    this.queryManager = new QueryManager(
      this.config.schema,
      this.local,
      this.streamProcessor,
      clientId,
      logger
    );
    this.auth = new AuthService(
      this.config.schema,
      this.remote,
      this.local,
      this.mutationManager,
      logger
    );
    this.sync = new SpookySync(this.config.schema, this.local, this.remote, clientId, logger);
    this.devTools = new DevToolsService(
      this.local,
      logger,
      this.config.schema,
      this.auth,
      this.queryManager
    );
    this.router = new RouterService(
      this.mutationManager.events,
      this.queryManager.eventsSystem,
      this.sync,
      this.queryManager,
      this.streamProcessor,
      this.devTools,
      this.auth,
      logger
    );
  }

  async init() {
    console.log('[Spooky] Init started');
    try {
      console.log('[Spooky] Connecting to local DB...');
      await this.local.connect();
      console.log('[Spooky] Local connected');

      console.log('[Spooky] Provisioning schema...');
      await this.migrator.provision(this.config.schemaSurql);
      console.log('[Spooky] Migrator provisioned');

      console.log('[Spooky] Connecting to remote DB...');
      await this.remote.connect();
      console.log('[Spooky] Remote connected');

      console.log('[Spooky] Initializing StreamProcessor...');
      await this.streamProcessor.init();
      console.log('[Spooky] StreamProcessor initialized');

      console.log('[Spooky] Initializing Auth...');
      await this.auth.init();
      console.log('[Spooky] Auth initialized');

      console.log('[Spooky] Initializing QueryManager...');
      await this.queryManager.init();
      console.log('[Spooky] QueryManager initialized');

      console.log('[Spooky] Initializing Sync...');
      await this.sync.init();
      console.log('[Spooky] Sync initialized');
    } catch (e) {
      console.error('[Spooky] Init failed', e);
      throw e;
    }
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
