import { DataModule } from './modules/data/index.js';
import {
  SpookyConfig,
  QueryTimeToLive,
  SpookyQueryResultPromise,
  PersistenceClient,
  MutationEvent,
} from './types.js';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index.js';
import { Surreal } from 'surrealdb';
import { SpookySync } from './modules/sync/index.js';
import {
  GetTable,
  InnerQuery,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from '@spooky/query-builder';

import { DevToolsService } from './modules/devtools/index.js';
import { createLogger } from './services/logger/index.js';
import { AuthService } from './modules/auth/index.js';
import { StreamProcessorService } from './services/stream-processor/index.js';
import { EventSystem } from './events/index.js';
import { CacheModule } from './modules/cache/index.js';
import { LocalStoragePersistenceClient } from './services/persistence/localstorage.js';
import { generateId, parseParams } from './utils/index.js';
import { SurrealDBPersistenceClient } from './services/persistence/surrealdb.js';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private persistenceClient: PersistenceClient;

  private migrator: LocalMigrator;
  private cache: CacheModule;
  private dataModule: DataModule<S>;
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
    const logger = createLogger(config.logLevel ?? 'info', config.otelEndpoint);
    this.logger = logger.child({ service: 'SpookyClient' });

    this.logger.info(
      {
        config: { ...config, schema: '[SchemaStructure]' },
        Category: 'spooky-client::SpookyClient::constructor',
      },
      'SpookyClient initialized'
    );

    this.local = new LocalDatabaseService(this.config.database, logger);
    this.remote = new RemoteDatabaseService(this.config.database, logger);

    if (config.persistenceClient === 'surrealdb') {
      this.persistenceClient = new SurrealDBPersistenceClient(this.local, logger);
    } else if (config.persistenceClient === 'localstorage' || !config.persistenceClient) {
      this.persistenceClient = new LocalStoragePersistenceClient(logger);
    } else {
      this.persistenceClient = config.persistenceClient;
    }

    this.streamProcessor = new StreamProcessorService(
      new EventSystem(['stream_update']),
      this.local,
      this.persistenceClient,
      logger
    );
    this.migrator = new LocalMigrator(this.local, logger);

    this.cache = new CacheModule(
      this.local,
      this.streamProcessor,
      (update) => {
        // Direct callback from cache to data module
        this.dataModule.onStreamUpdate(update);
      },
      logger
    );

    this.dataModule = new DataModule(this.cache, this.local, this.config.schema, logger);

    // Initialize Auth
    this.auth = new AuthService(this.config.schema, this.remote, this.persistenceClient, logger);

    // Initialize Sync
    this.sync = new SpookySync(this.local, this.remote, this.cache, this.dataModule, this.logger);

    // Initialize DevTools
    this.devTools = new DevToolsService(
      this.local,
      this.remote,
      logger,
      this.config.schema,
      this.auth,
      this.dataModule
    );

    // Register DevTools as a receiver for stream updates
    this.streamProcessor.addReceiver(this.devTools);

    // Wire up callbacks instead of events
    this.setupCallbacks();
  }

  /**
   * Setup direct callbacks instead of event subscriptions
   */
  private setupCallbacks() {
    // Mutation callback for sync
    this.dataModule.onMutation((mutations: MutationEvent[]) => {
      // Notify DevTools
      this.devTools.onMutation(mutations);

      // Enqueue in Sync
      if (mutations.length > 0) {
        this.sync.enqueueMutation(mutations as any);
      }
    });

    // Sync events for incoming updates
    this.sync.events.subscribe('SYNC_QUERY_UPDATED', (event: any) => {
      this.devTools.logEvent('SYNC_QUERY_UPDATED', event.payload);
    });

    // Database events for DevTools
    this.local.getEvents().subscribe('DATABASE_LOCAL_QUERY', (event: any) => {
      this.devTools.logEvent('LOCAL_QUERY', event.payload);
    });

    this.remote.getEvents().subscribe('DATABASE_REMOTE_QUERY', (event: any) => {
      this.devTools.logEvent('REMOTE_QUERY', event.payload);
    });
  }

  async init() {
    this.logger.info(
      { Category: 'spooky-client::SpookyClient::init' },
      'SpookyClient initialization started'
    );
    try {
      const clientId = this.config.clientId ?? (await this.loadOrGenerateClientId());
      this.persistClientId(clientId);
      this.logger.debug(
        { clientId, Category: 'spooky-client::SpookyClient::init' },
        'Client ID loaded'
      );

      await this.local.connect();
      this.logger.debug(
        { Category: 'spooky-client::SpookyClient::init' },
        'Local database connected'
      );

      await this.migrator.provision(this.config.schemaSurql);
      this.logger.debug({ Category: 'spooky-client::SpookyClient::init' }, 'Schema provisioned');

      await this.remote.connect();
      this.logger.debug(
        { Category: 'spooky-client::SpookyClient::init' },
        'Remote database connected'
      );

      await this.streamProcessor.init();
      this.logger.debug(
        { Category: 'spooky-client::SpookyClient::init' },
        'StreamProcessor initialized'
      );

      await this.auth.init();
      this.logger.debug({ Category: 'spooky-client::SpookyClient::init' }, 'Auth initialized');

      await this.dataModule.init();
      this.logger.debug(
        { Category: 'spooky-client::SpookyClient::init' },
        'DataModule initialized'
      );

      await this.sync.init(clientId);
      this.logger.debug({ Category: 'spooky-client::SpookyClient::init' }, 'Sync initialized');

      this.logger.info(
        { Category: 'spooky-client::SpookyClient::init' },
        'SpookyClient initialization completed successfully'
      );
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client::SpookyClient::init' },
        'SpookyClient initialization failed'
      );
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
        hash: await this.initQuery(table, q, ttl),
      }),
      options
    );
  }

  private async initQuery<Table extends TableNames<S>>(
    table: Table,
    q: InnerQuery<any, any, any>,
    ttl: QueryTimeToLive
  ) {
    const tableSchema = this.config.schema.tables.find((t) => t.name === table);
    if (!tableSchema) {
      throw new Error(`Table ${table} not found`);
    }

    const hash = await this.dataModule.query(
      table,
      q.selectQuery.query,
      parseParams(tableSchema.columns, q.selectQuery.vars ?? {}),
      ttl
    );

    await this.sync.enqueueDownEvent({
      type: 'register',
      payload: {
        hash,
      },
    });

    return hash;
  }

  async queryRaw(sql: string, params: Record<string, any>, ttl: QueryTimeToLive) {
    const tableName = sql.split('FROM ')[1].split(' ')[0];
    return this.dataModule.query(tableName, sql, params, ttl);
  }

  async subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options?: { immediate?: boolean }
  ): Promise<() => void> {
    return this.dataModule.subscribe(queryHash, callback, options);
  }

  create(id: string, data: Record<string, unknown>) {
    return this.dataModule.create(id, data);
  }

  update(table: string, id: string, data: Record<string, unknown>) {
    return this.dataModule.update(table, id, data);
  }

  delete(table: string, id: string) {
    return this.dataModule.delete(table, id);
  }

  async useRemote<T>(fn: (client: Surreal) => Promise<T> | T): Promise<T> {
    return fn(this.remote.getClient());
  }

  private persistClientId(id: string) {
    try {
      this.persistenceClient.set('spooky_client_id', id);
    } catch (e) {
      this.logger.warn(
        { error: e, Category: 'spooky-client::SpookyClient::persistClientId' },
        'Failed to persist client ID'
      );
    }
  }

  private async loadOrGenerateClientId(): Promise<string> {
    const clientId = await this.persistenceClient.get<string>('spooky_client_id');

    if (clientId) {
      return clientId;
    }

    const newId = generateId();
    await this.persistClientId(newId);
    return newId;
  }
}
