import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { RecordVersionArray } from '../../types';
import { createSyncEventSystem, SyncEventTypes, SyncQueueEventTypes } from './events/index';
import type { Logger } from '../../services/logger/index';
import type { DownEvent, UpEvent} from './queue/index';
import { DownQueue, UpQueue } from './queue/index';
import type { RecordId, Uuid } from 'surrealdb';
import { ArraySyncer, createDiffFromDbOp } from './utils';
import { SyncEngine } from './engine';
import { SyncScheduler } from './scheduler';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { CacheModule } from '../cache/index';
import type { DataModule } from '../data/index';
import { encodeRecordId, extractTablePart, surql } from '../../utils/index';

/**
 * The main synchronization engine for Sp00ky.
 * Handles the bidirectional synchronization between the local database and the remote backend.
 * Uses a queue-based architecture with 'up' (local to remote) and 'down' (remote to local) queues.
 * @template S The schema structure type.
 */
export class Sp00kySync<S extends SchemaStructure> {
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

  get pendingMutationCount(): number {
    return this.upQueue.size;
  }

  subscribeToPendingMutations(cb: (count: number) => void): () => void {
    const id1 = this.upQueue.events.subscribe(
      SyncQueueEventTypes.MutationEnqueued,
      (event) => cb(event.payload.queueSize)
    );
    const id2 = this.upQueue.events.subscribe(
      SyncQueueEventTypes.MutationDequeued,
      (event) => cb(event.payload.queueSize)
    );
    return () => {
      this.upQueue.events.unsubscribe(id1);
      this.upQueue.events.unsubscribe(id2);
    };
  }

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private cache: CacheModule,
    private dataModule: DataModule<S>,
    private schema: S,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'Sp00kySync' });
    this.upQueue = new UpQueue(this.local, this.logger);
    this.downQueue = new DownQueue(this.local, this.logger);
    this.syncEngine = new SyncEngine(this.remote, this.cache, this.schema, this.logger);
    this.scheduler = new SyncScheduler(
      this.upQueue,
      this.downQueue,
      this.processUpEvent.bind(this),
      this.processDownEvent.bind(this),
      this.logger,
      this.handleRollback.bind(this)
    );
  }

  /**
   * Initializes the synchronization system.
   * Starts the scheduler and initiates the initial sync cycles.
   * @param clientId The unique identifier for this client instance.
   * @throws Error if already initialized.
   */
  public async init(clientId: string) {
    if (this.isInit) throw new Error('Sp00kySync is already initialized');
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
      { clientId: this.clientId, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
      'Starting ref live queries'
    );

    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _00_list_ref'
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug(
        { message, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
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
          { err, Category: 'sp00ky-client::Sp00kySync::startRefLiveQueries' },
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
    if (action === 'DELETE') {
      this.logger.debug(
        {
          queryId: queryId.toString(),
          recordId: recordId.toString(),
          Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
        },
        'Ignoring DELETE on list_ref — should not happen'
      );
      return;
    }

    const existing = this.dataModule.getQueryById(queryId);

    if (!existing) {
      this.logger.warn(
        {
          queryId: queryId.toString(),
          Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
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
        Category: 'sp00ky-client::Sp00kySync::handleRemoteListRefChange',
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
      { event, Category: 'sp00ky-client::Sp00kySync::processUpEvent' },
      'Processing up event'
    );
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
          { event, Category: 'sp00ky-client::Sp00kySync::processUpEvent' },
          'processUpEvent unknown event type'
        );
        return;
    }
  }

  private async handleRollback(event: UpEvent, error: Error): Promise<void> {
    const recordId = encodeRecordId(event.record_id);
    const tableName =
      event.type === 'create' && event.tableName
        ? event.tableName
        : extractTablePart(recordId);

    this.logger.warn(
      {
        type: event.type,
        recordId,
        tableName,
        error: error.message,
        Category: 'sp00ky-client::Sp00kySync::handleRollback',
      },
      'Rolling back failed mutation'
    );

    switch (event.type) {
      case 'create':
        await this.dataModule.rollbackCreate(event.record_id, tableName);
        break;
      case 'update':
        if (event.beforeRecord) {
          await this.dataModule.rollbackUpdate(event.record_id, tableName, event.beforeRecord);
        } else {
          this.logger.warn(
            {
              recordId,
              Category: 'sp00ky-client::Sp00kySync::handleRollback',
            },
            'Cannot rollback update: no beforeRecord available. Down-sync will reconcile.'
          );
        }
        break;
      case 'delete':
        this.logger.warn(
          {
            recordId,
            Category: 'sp00ky-client::Sp00kySync::handleRollback',
          },
          'Delete rollback not implemented. Down-sync will reconcile.'
        );
        break;
    }

    this.events.emit(SyncEventTypes.MutationRolledBack, {
      eventType: event.type,
      recordId,
      error: error.message,
    });
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug(
      { event, Category: 'sp00ky-client::Sp00kySync::processDownEvent' },
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
        { hash, Category: 'sp00ky-client::Sp00kySync::syncQuery' },
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
        { queryHash, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'Register Query state'
      );
      await this.createRemoteQuery(queryHash);
      await this.syncQuery(queryHash);
      // Always notify after sync completes — handles empty result sets
      // where no stream updates fire but the UI needs to stop loading
      await this.dataModule.notifyQuerySynced(queryHash);
    } catch (e) {
      this.logger.error(
        { err: e, Category: 'sp00ky-client::Sp00kySync::registerQuery' },
        'registerQuery error'
      );
      throw e;
    }
  }

  private async createRemoteQuery(queryHash: string) {
    const queryState = this.dataModule.getQueryByHash(queryHash);

    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::Sp00kySync::createRemoteQuery' },
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
      surql.selectByFieldsAnd('_00_list_ref', ['in'], ['out', 'version']),
      {
        in: queryState.config.id,
      }
    );

    this.logger.trace(
      {
        queryId: encodeRecordId(queryState.config.id),
        items,
        Category: 'sp00ky-client::Sp00kySync::createRemoteQuery',
      },
      'Got query record version array from remote'
    );

    const array: RecordVersionArray = items.map((item) => [encodeRecordId(item.out), item.version]);

    this.logger.debug(
      {
        queryId: encodeRecordId(queryState.config.id),
        array,
        Category: 'sp00ky-client::Sp00kySync::createRemoteQuery',
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
        { queryHash, Category: 'sp00ky-client::Sp00kySync::heartbeatQuery' },
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
        { queryHash, Category: 'sp00ky-client::Sp00kySync::cleanupQuery' },
        'Query to register not found'
      );
      throw new Error('Query to register not found');
    }
    await this.remote.query(`DELETE $id`, {
      id: queryState.config.id,
    });
  }
}
