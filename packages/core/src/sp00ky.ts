import { DataModule } from './modules/data/index';
import type {
  Sp00kyConfig,
  QueryTimeToLive,
  Sp00kyQueryResultPromise,
  PersistenceClient,
  UpdateOptions,
  RunOptions} from './types';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index';
import type { UpEvent } from './modules/sync/index';
import { Sp00kySync } from './modules/sync/index';
import type {
  GetTable,
  InnerQuery,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
  BucketNames,
  BackendNames,
  BackendRoutes,
  RoutePayload} from '@spooky-sync/query-builder';
import {
  QueryBuilder
} from '@spooky-sync/query-builder';

import { DevToolsService } from './modules/devtools/index';
import { createLogger } from './services/logger/index';
import { AuthService } from './modules/auth/index';
import { StreamProcessorService } from './services/stream-processor/index';
import { EventSystem } from './events/index';
import { CacheModule } from './modules/cache/index';
import { LocalStoragePersistenceClient } from './services/persistence/localstorage';
import { generateId, parseParams } from './utils/index';
import { SurrealDBPersistenceClient } from './services/persistence/surrealdb';
import { ResilientPersistenceClient } from './services/persistence/resilient';

export class BucketHandle {
  constructor(private bucketName: string, private remote: RemoteDatabaseService) {}

  async put(path: string, content: string | Uint8Array | Blob): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${path}".put($content);`, { content });
  }

  async get(path: string): Promise<unknown> {
    const [result] = await this.remote.query<[unknown]>(`RETURN f"${this.bucketName}:/${path}".get();`);
    return result;
  }

  async delete(path: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${path}".delete();`);
  }

  async exists(path: string): Promise<boolean> {
    const [result] = await this.remote.query<[boolean]>(`RETURN f"${this.bucketName}:/${path}".exists();`);
    return result;
  }

  async head(path: string): Promise<Record<string, unknown>> {
    const [result] = await this.remote.query<[Record<string, unknown>]>(`RETURN f"${this.bucketName}:/${path}".head();`);
    return result;
  }

  async copy(sourcePath: string, targetPath: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${sourcePath}".copy($target);`, { target: targetPath });
  }

  async rename(sourcePath: string, targetPath: string): Promise<void> {
    await this.remote.query(`RETURN f"${this.bucketName}:/${sourcePath}".rename($target);`, { target: targetPath });
  }

  async list(prefix?: string): Promise<string[]> {
    const p = prefix ?? '';
    const [result] = await this.remote.query<[string[]]>(`RETURN f"${this.bucketName}:/${p}".list();`);
    return result;
  }
}

export class Sp00kyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private persistenceClient: PersistenceClient;

  private migrator: LocalMigrator;
  private cache: CacheModule;
  private dataModule: DataModule<S>;
  private sync: Sp00kySync<S>;
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

  get pendingMutationCount(): number {
    return this.sync.pendingMutationCount;
  }

  subscribeToPendingMutations(cb: (count: number) => void): () => void {
    return this.sync.subscribeToPendingMutations(cb);
  }

  constructor(private config: Sp00kyConfig<S>) {
    const logger = createLogger(config.logLevel ?? 'info', config.otelTransmit);
    this.logger = logger.child({ service: 'Sp00kyClient' });

    this.logger.info(
      {
        config: { ...config, schema: '[SchemaStructure]' },
        Category: 'sp00ky-client::Sp00kyClient::constructor',
      },
      'Sp00kyClient initialized'
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

    this.persistenceClient = new ResilientPersistenceClient(this.persistenceClient, logger);

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

    this.dataModule = new DataModule(
      this.cache,
      this.local,
      this.config.schema,
      logger,
      this.config.streamDebounceTime
    );

    // Initialize Auth
    this.auth = new AuthService(this.config.schema, this.remote, this.persistenceClient, logger);

    // Initialize Sync
    this.sync = new Sp00kySync(this.local, this.remote, this.cache, this.dataModule, this.config.schema, this.logger);

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
    this.dataModule.onMutation((mutations: UpEvent[]) => {
      // Notify DevTools
      this.devTools.onMutation(mutations);

      // Enqueue in Sync
      if (mutations.length > 0) {
        this.sync.enqueueMutation(mutations);
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
      { Category: 'sp00ky-client::Sp00kyClient::init' },
      'Sp00kyClient initialization started'
    );
    try {
      const clientId = this.config.clientId ?? (await this.loadOrGenerateClientId());
      this.persistClientId(clientId);
      this.logger.debug(
        { clientId, Category: 'sp00ky-client::Sp00kyClient::init' },
        'Client ID loaded'
      );

      await this.local.connect();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'Local database connected'
      );

      await this.migrator.provision(this.config.schemaSurql);
      this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Schema provisioned');

      await this.remote.connect();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'Remote database connected'
      );

      await this.streamProcessor.init();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'StreamProcessor initialized'
      );

      await this.auth.init();
      this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Auth initialized');

      await this.dataModule.init();
      this.logger.debug(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'DataModule initialized'
      );

      await this.sync.init(clientId);
      this.logger.debug({ Category: 'sp00ky-client::Sp00kyClient::init' }, 'Sync initialized');

      this.logger.info(
        { Category: 'sp00ky-client::Sp00kyClient::init' },
        'Sp00kyClient initialization completed successfully'
      );
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::Sp00kyClient::init' },
        'Sp00kyClient initialization failed'
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
  ): QueryBuilder<S, Table, Sp00kyQueryResultPromise> {
    return new QueryBuilder<S, Table, Sp00kyQueryResultPromise>(
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

  run<
    B extends BackendNames<S>,
    R extends BackendRoutes<S, B>,
  >(backend: B, path: R, payload: RoutePayload<S, B, R>, options?: RunOptions) {
    return this.dataModule.run(backend, path, payload, options);
  }

  bucket<B extends BucketNames<S>>(name: B): BucketHandle {
    return new BucketHandle(name, this.remote);
  }

  create(id: string, data: Record<string, unknown>) {
    return this.dataModule.create(id, data);
  }

  update(table: string, id: string, data: Record<string, unknown>, options?: UpdateOptions) {
    return this.dataModule.update(table, id, data, options);
  }

  delete(table: string, id: string) {
    return this.dataModule.delete(table, id);
  }

  async useRemote<T>(fn: (client: Surreal) => Promise<T> | T): Promise<T> {
    return fn(this.remote.getClient());
  }

  private persistClientId(id: string) {
    try {
      this.persistenceClient.set('sp00ky_client_id', id);
    } catch (e) {
      this.logger.warn(
        { error: e, Category: 'sp00ky-client::Sp00kyClient::persistClientId' },
        'Failed to persist client ID'
      );
    }
  }

  private async loadOrGenerateClientId(): Promise<string> {
    const clientId = await this.persistenceClient.get<string>('sp00ky_client_id');

    if (clientId) {
      return clientId;
    }

    const newId = generateId();
    await this.persistClientId(newId);
    return newId;
  }
}
