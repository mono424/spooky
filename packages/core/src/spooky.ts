import { DataManager } from './modules/data/index.js';
import { SpookyConfig, QueryTimeToLive, SpookyQueryResultPromise } from './types.js';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index.js';
import { Surreal } from 'surrealdb';
import { SpookySync } from './modules/sync/index.js';
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from '@spooky/query-builder';

import { DevToolsService } from './modules/devtools/index.js';
import { createLogger } from './services/logger/index.js';
import { AuthService } from './modules/auth/index.js';
import { AuthEventTypes } from './modules/auth/index.js';
import { MutationEventTypes } from './modules/data/events/mutation.js';
import { QueryEventTypes } from './modules/data/events/query.js';
import { StreamProcessorService } from './services/stream-processor/index.js';
import { SyncEventTypes, UpEvent } from './modules/sync/index.js';
import { EventSystem } from './events/index.js';
import { DatabaseEventTypes } from './services/database/index.js';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private migrator: LocalMigrator;
  private dataManager: DataManager<S>;
  private sync: SpookySync<S>;
  private devTools: DevToolsService;

  private logger: ReturnType<typeof createLogger>;
  public auth: AuthService<S>;
  public streamProcessor: StreamProcessorService;

  get remoteClient() {
    return this.remote.getClient();
  }

  get localClient() {
    return this.local.getClient();
  }

  constructor(private config: SpookyConfig<S>) {
    const logger = createLogger(config.logLevel ?? 'info');
    this.logger = logger.child({ service: 'SpookyClient' });

    const clientId = this.config.clientId ?? this.loadOrGenerateClientId();
    this.persistClientId(clientId);

    this.logger.info(
      { config: { ...config, schema: '[SchemaStructure]' } },
      '[SpookyClient] Constructor called'
    );
    this.local = new LocalDatabaseService(this.config.database, logger);
    this.remote = new RemoteDatabaseService(this.config.database, logger);
    this.streamProcessor = new StreamProcessorService(
      new EventSystem(['stream_update']),
      this.local,
      logger
    );
    this.migrator = new LocalMigrator(this.local, logger);
    this.dataManager = new DataManager(
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
      this.dataManager,
      logger
    );
    this.sync = new SpookySync(
      this.config.schema,
      this.local,
      this.remote,
      this.streamProcessor,
      clientId,
      logger
    );
    this.devTools = new DevToolsService(
      this.local,
      logger,
      this.config.schema,
      this.auth,
      this.dataManager
    );
  }

  async init() {
    this.logger.info('[SpookyClient] Init started');
    try {
      this.logger.debug('[SpookyClient] Connecting to local DB...');
      await this.local.connect();
      this.logger.debug('[SpookyClient] Local connected');

      this.logger.debug('[SpookyClient] Provisioning schema...');
      await this.migrator.provision(this.config.schemaSurql);
      this.logger.debug('[SpookyClient] Migrator provisioned');

      this.logger.debug('[SpookyClient] Connecting to remote DB...');
      await this.remote.connect();
      this.logger.debug('[SpookyClient] Remote connected');

      this.logger.debug('[SpookyClient] Initializing StreamProcessor...');
      await this.streamProcessor.init();
      this.logger.debug('[SpookyClient] StreamProcessor initialized');

      this.logger.debug('[SpookyClient] Initializing Auth...');
      await this.auth.init();
      this.logger.debug('[SpookyClient] Auth initialized');

      this.logger.debug('[SpookyClient] Initializing DataManager...');
      await this.dataManager.init();
      this.logger.debug('[SpookyClient] DataManager initialized');

      this.logger.debug('[SpookyClient] Initializing Sync...');
      await this.sync.init();
      this.logger.debug('[SpookyClient] Sync initialized');

      // -------------------------------------------------------------------------
      // EVENT ROUTING
      // -------------------------------------------------------------------------

      // --- Mutation Events ---
      this.dataManager.mutationEvents.subscribe(MutationEventTypes.MutationCreated, (event) => {
        const payload = event.payload;
        // 1. Notify DevTools
        this.devTools.onMutation(payload);

        // 2. Enqueue in Sync (filter out localOnly)
        const mutationsToSync = payload.filter((e: UpEvent) => !e.localOnly);
        if (mutationsToSync.length > 0) {
          this.sync.enqueueMutation(mutationsToSync);
        }

        // 3. Ingest into StreamProcessor
        for (const element of payload) {
          const tableName = element.record_id.table.toString();
          // Use full record for create/update, fall back to data if record not available
          const recordData =
            element.type === 'delete' ? undefined : ((element as any).record ?? element.data);
          this.streamProcessor.ingest(
            tableName,
            element.type,
            element.record_id.toString(),
            recordData
          );
        }
      });

      // --- Query Events ---
      this.dataManager.queryEvents.subscribe(QueryEventTypes.IncantationInitialized, (event) => {
        this.devTools.onQueryInitialized(event.payload);
        this.sync.enqueueDownEvent({ type: 'register', payload: event.payload });
      });

      this.dataManager.queryEvents.subscribe(
        QueryEventTypes.IncantationRemoteHashUpdate,
        (event) => {
          this.devTools.logEvent('QUERY_REMOTE_HASH_UPDATE', event.payload);
          this.sync.enqueueDownEvent({ type: 'sync', payload: event.payload });
        }
      );

      this.dataManager.queryEvents.subscribe(QueryEventTypes.IncantationTTLHeartbeat, (event) => {
        this.devTools.logEvent('QUERY_TTL_HEARTBEAT', event.payload);
        this.sync.enqueueDownEvent({ type: 'heartbeat', payload: event.payload });
      });

      this.dataManager.queryEvents.subscribe(QueryEventTypes.IncantationCleanup, (event) => {
        this.devTools.logEvent('QUERY_CLEANUP', event.payload);
        this.sync.enqueueDownEvent({ type: 'cleanup', payload: event.payload });
      });

      this.dataManager.queryEvents.subscribe(QueryEventTypes.IncantationUpdated, (event) => {
        this.devTools.onQueryUpdated(event.payload);
      });

      // --- Sync Events ---
      this.sync.events.subscribe(SyncEventTypes.IncantationUpdated, (event: any) => {
        this.devTools.logEvent('SYNC_INCANTATION_UPDATED', event.payload);
        this.dataManager.handleIncomingUpdate(event.payload);
      });

      this.sync.events.subscribe(SyncEventTypes.RemoteDataIngested, (event: any) => {
        // Just logging or devtools update if needed
      });

      // --- StreamProcessor Events ---
      this.streamProcessor.events.subscribe('stream_update', (event: any) => {
        const payload = event.payload;
        this.devTools.onStreamUpdate(payload);
        if (Array.isArray(payload)) {
          for (const update of payload) {
            this.dataManager.handleStreamUpdate(update);
          }
        }
      });

      // --- Auth Events ---
      this.auth.eventSystem.subscribe(AuthEventTypes.AuthStateChanged, (event) => {
        // Handle auth state changes if needed (e.g. re-subscribe, clear cache)
      });

      // --- Database Events ---
      this.local.getEvents().subscribe('DATABASE_LOCAL_QUERY', (event: any) => {
        this.devTools.logEvent('LOCAL_QUERY', event.payload);
      });

      this.remote.getEvents().subscribe('DATABASE_REMOTE_QUERY', (event: any) => {
        this.devTools.logEvent('REMOTE_QUERY', event.payload);
      });
    } catch (e) {
      this.logger.error({ error: e }, '[SpookyClient] Init failed');
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
        hash: await this.dataManager.query(
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
    return this.dataManager.query(tableName, sql, params, ttl);
  }

  async subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options?: { immediate?: boolean }
  ): Promise<() => void> {
    return this.dataManager.subscribe(queryHash, callback, options);
  }

  create(id: string, data: Record<string, unknown>) {
    return this.dataManager.create(id, data);
  }

  update(table: string, id: string, data: Record<string, unknown>) {
    return this.dataManager.update(table, id, data);
  }

  delete(table: string, id: string) {
    return this.dataManager.delete(table, id);
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
      this.logger.warn({ error: e }, '[SpookyClient] Failed to persist client ID');
    }
  }

  private loadOrGenerateClientId(): string {
    try {
      if (typeof localStorage !== 'undefined') {
        const stored = localStorage.getItem('spooky_client_id');
        if (stored) return stored;
      }
    } catch (e) {
      this.logger.warn({ error: e }, '[SpookyClient] Failed to load client ID');
    }

    const newId = crypto.randomUUID();
    this.persistClientId(newId);
    return newId;
  }
}
