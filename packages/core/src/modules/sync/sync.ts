import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import { MutationEvent, RecordVersionArray } from '../../types';
import { createSyncEventSystem } from './events/index';
import { Logger } from '../../services/logger/index';
import { DownEvent, DownQueue, UpEvent, UpQueue } from './queue/index';
import { RecordId, Uuid } from 'surrealdb';
import { ArraySyncer, createDiffFromDbOp } from './utils';
import { SyncEngine } from './engine';
import { SyncScheduler } from './scheduler';
import { SchemaStructure } from '@spooky/query-builder';
import { CacheModule } from '../cache/index';
import { DataModule } from '../data/index';
import { encodeRecordId, parseDuration, surql } from '../../utils/index';

/**
 * The main synchronization engine for Spooky.
 * Handles the bidirectional synchronization between the local database and the remote backend.
 * Uses a queue-based architecture with 'up' (local to remote) and 'down' (remote to local) queues.
 * @template S The schema structure type.
 */
export class SpookySync<S extends SchemaStructure> {
  private clientId: string = '';
  private upQueue: UpQueue;
  private downQueue: DownQueue;
  private isInit: boolean = false;
  private logger: Logger;
  private syncEngine: SyncEngine;
  private scheduler: SyncScheduler;
  public events = createSyncEventSystem();

  get isSyncing() {
    return this.scheduler.isSyncing;
  }

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private cache: CacheModule,
    private dataModule: DataModule<S>,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'SpookySync' });
    this.upQueue = new UpQueue(this.local, this.logger);
    this.downQueue = new DownQueue(this.local, this.logger);
    this.syncEngine = new SyncEngine(this.remote, this.cache, this.logger);
    this.scheduler = new SyncScheduler(
      this.upQueue,
      this.downQueue,
      this.processUpEvent.bind(this),
      this.processDownEvent.bind(this),
      this.logger
    );
  }

  /**
   * Initializes the synchronization system.
   * Starts the scheduler and initiates the initial sync cycles.
   * @param clientId The unique identifier for this client instance.
   * @throws Error if already initialized.
   */
  public async init(clientId: string) {
    if (this.isInit) throw new Error('SpookySync is already initialized');
    this.clientId = clientId;
    this.isInit = true;
    await this.scheduler.init();
    void this.scheduler.syncUp();
    void this.scheduler.syncUp();
    void this.scheduler.syncDown();
    void this.startRefLiveQueries();
  }

  private async startRefLiveQueries() {
    this.logger.debug(
      { clientId: this.clientId, Category: 'spooky-client::SpookySync::startRefLiveQueries' },
      'Starting ref live queries'
    );

    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _spooky_list_ref WHERE clientId = $clientId',
      {
        clientId: this.clientId,
      }
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug(
        { message, Category: 'spooky-client::SpookySync::startRefLiveQueries' },
        'Live update received'
      );
      if (message.action === 'KILLED') return;
      this.handleRemoteListRefChange(
        message.action,
        message.value.in as RecordId<string>,
        message.value.out as RecordId<string>,
        message.value.version as number
      ).catch((err) => {
        this.logger.error(
          { err, Category: 'spooky-client::SpookySync::startRefLiveQueries' },
          'Error handling remote list ref change'
        );
      });
    });
  }

  private async handleRemoteListRefChange(
    action: 'CREATE' | 'UPDATE' | 'DELETE',
    queryId: RecordId,
    recordId: RecordId,
    version: number
  ) {
    const existing = this.dataModule.getQueryById(queryId);

    if (!existing) {
      this.logger.warn(
        {
          queryId: queryId.toString(),
          Category: 'spooky-client::SpookySync::handleRemoteListRefChange',
        },
        'Received remote update for unknown local query'
      );
      return;
    }

    const { localArray } = existing.config;

    this.logger.debug(
      {
        action,
        queryId,
        recordId,
        version,
        localArray,
        Category: 'spooky-client::SpookySync::handleRemoteListRefChange',
      },
      'Live update is being processed'
    );
    const diff = createDiffFromDbOp(action, recordId, version, localArray);
    await this.syncEngine.syncRecords(diff);
  }

  /**
   * Enqueues a 'down' event (from remote to local) for processing.
   * @param event The DownEvent to enqueue.
   */
  public enqueueDownEvent(event: DownEvent) {
    this.scheduler.enqueueDownEvent(event);
  }

  private async processUpEvent(event: UpEvent) {
    this.logger.debug(
      { event, Category: 'spooky-client::SpookySync::processUpEvent' },
      'Processing up event'
    );
    console.log('xx1', event);
    switch (event.type) {
      case 'create':
        const dataKeys = Object.keys(event.data).map((key) => ({ key, variable: `data_${key}` }));
        const prefixedParams = Object.fromEntries(
          dataKeys.map(({ key, variable }) => [variable, event.data[key]])
        );
        const query = surql.seal(surql.createSet('id', dataKeys));
        await this.remote.query(query, {
          id: event.record_id,
          ...prefixedParams,
        });
        break;
      case 'update':
        await this.remote.query(`UPDATE $id MERGE $data`, {
          id: event.record_id,
          data: event.data,
        });
        break;
      case 'delete':
        await this.remote.query(`DELETE $id`, {
          id: event.record_id,
        });
        break;
      default:
        this.logger.error(
          { event, Category: 'spooky-client::SpookySync::processUpEvent' },
          'processUpEvent unknown event type'
        );
        return;
    }
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug(
      { event, Category: 'spooky-client::SpookySync::processDownEvent' },
      'Processing down event'
    );
    switch (event.type) {
      case 'register':
        return this.registerQuery(event.payload.hash);
      case 'sync':
        return this.syncQuery(event.payload.hash);
      case 'heartbeat':
        return this.heartbeatQuery(event.payload.hash);
      case 'cleanup':
        return this.cleanupQuery(event.payload.hash);
    }
  }

  /**
   * Synchronizes a specific query by hash.
   * Compares local and remote version arrays and fetches differences.
   * @param hash The hash of the query to sync.
   */
  public async syncQuery(hash: string) {
    const queryState = this.dataModule.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn(
        { hash, Category: 'spooky-client::SpookySync::syncQuery' },
        'Query not found'
      );
      return;
    }

    const diff = new ArraySyncer(
      queryState.config.localArray,
      queryState.config.remoteArray
    ).nextSet();

    if (!diff) {
      return;
    }
    return this.syncEngine.syncRecords(diff);
  }

  /**
   * Enqueues a list of mutations (up events) to be sent to the remote.
   * @param mutations Array of UpEvents (create/update/delete) to enqueue.
   */
  public async enqueueMutation(mutations: UpEvent[]) {
    this.scheduler.enqueueMutation(mutations);
  }

  private async registerQuery(queryHash: string) {
    try {
      this.logger.debug(
        { queryHash, Category: 'spooky-client::SpookySync::registerQuery' },
        'Register Query state'
      );
      await this.createRemoteQuery(queryHash);
      await this.syncQuery(queryHash);
    } catch (e) {
      this.logger.error(
        { err: e, Category: 'spooky-client::SpookySync::registerQuery' },
        'registerQuery error'
      );
      throw e;
    }
  }

  private async createRemoteQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);

    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'spooky-client::SpookySync::createRemoteQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    // Delegate to remote function which handles DBSP registration & persistence
    await this.remote.query('fn::query::register($config)', {
      config: {
        clientId: this.clientId,
        id: queryState.config.id,
        surql: queryState.config.surql,
        params: queryState.config.params,
        ttl: queryState.config.ttl,
      },
    });

    const [items] = await this.remote.query<[{ out: RecordId<string>; version: number }[]]>(
      surql.selectByFieldsAnd('_spooky_list_ref', ['in'], ['out', 'version']),
      {
        in: queryState.config.id,
      }
    );

    this.logger.trace(
      {
        queryId: encodeRecordId(queryState.config.id),
        items,
        Category: 'spooky-client::SpookySync::createRemoteQuery',
      },
      'Got query record version array from remote'
    );

    const array: RecordVersionArray = items.map((item) => [encodeRecordId(item.out), item.version]);

    this.logger.debug(
      {
        queryId: encodeRecordId(queryState.config.id),
        array,
        Category: 'spooky-client::SpookySync::createRemoteQuery',
      },
      'createdRemoteQuery'
    );

    if (array) {
      /// Incantation existed already
      await this.dataModule.updateQueryRemoteArray(queryHash, array);
    }
  }

  private async heartbeatQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'spooky-client::SpookySync::heartbeatQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    await this.remote.query('fn::query::heartbeat($id)', {
      id: queryState.config.id,
    });
  }

  private async cleanupQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'spooky-client::SpookySync::cleanupQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    await this.remote.query(`DELETE $id`, {
      id: queryState.config.id,
    });
  }
}
