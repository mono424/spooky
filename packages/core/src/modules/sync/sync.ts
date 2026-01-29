import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { RecordVersionArray } from '../../types.js';
import { createSyncEventSystem } from './events/index.js';
import { Logger } from '../../services/logger/index.js';
import { DownEvent, DownQueue, UpEvent, UpQueue } from './queue/index.js';
import { RecordId, Uuid } from 'surrealdb';
import { ArraySyncer, createDiffFromDbOp } from './utils.js';
import { SyncEngine } from './engine.js';
import { SyncScheduler } from './scheduler.js';
import { SchemaStructure } from '@spooky/query-builder';
import { CacheModule } from '../cache/index.js';
import { DataModule } from '../data/index.js';
import { encodeRecordId } from '../../utils/index.js';

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
    this.logger.debug({ clientId: this.clientId }, 'Starting ref live queries');

    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _spooky_list_ref WHERE clientId = $clientId',
      {
        clientId: this.clientId,
      }
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug({ message }, 'Live update received');
      if (message.action === 'KILLED') return;
      this.handleRemoteListRefChange(
        message.action,
        message.value.in as RecordId<string>,
        message.value.out as RecordId<string>,
        message.value.version as number
      ).catch((err) => {
        this.logger.error({ err }, 'Error handling remote list ref change');
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
        { queryId: queryId.toString() },
        'Received remote update for unknown local query'
      );
      return;
    }

    const { localArray } = existing.config;
    const diff = createDiffFromDbOp(action, recordId, version, localArray);
    await this.syncEngine.syncRecords(diff);
  }

  public enqueueDownEvent(event: DownEvent) {
    this.scheduler.enqueueDownEvent(event);
  }

  private async processUpEvent(event: UpEvent) {
    switch (event.type) {
      case 'create':
        await this.remote.query(`CREATE $id CONTENT $data`, {
          id: event.record_id,
          data: event.data,
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
        this.logger.error({ event }, 'processUpEvent unknown event type');
        return;
    }
  }

  private async processDownEvent(event: DownEvent) {
    this.logger.debug({ event }, 'Processing down event');
    switch (event.type) {
      case 'register':
        return this.registerQuery(event.payload.queryId);
      case 'sync':
        return this.syncQuery(event.payload.queryId);
      case 'heartbeat':
        return this.heartbeatQuery(event.payload.queryId);
      case 'cleanup':
        return this.cleanupQuery(event.payload.queryId);
    }
  }

  public async syncQuery(hash: string) {
    const queryState = this.dataModule.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn({ hash }, 'Query not found');
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

  public async enqueueMutation(mutations: any[]) {
    this.scheduler.enqueueMutation(mutations);
  }

  private async registerQuery(queryId: string) {
    try {
      this.logger.debug({ queryId }, 'Register Query state');
      await this.createRemoteQuery(queryId);
      await this.syncQuery(queryId);
    } catch (e) {
      console.log('registerQuery error', JSON.stringify(e));
      this.logger.error({ err: e }, 'registerQuery error');
      throw e;
    }
  }

  private async createRemoteQuery(hash: string) {
    const queryState = this.dataModule.getQueryByHash(hash);

    if (!queryState) {
      this.logger.warn({ hash }, 'Query to register not found');
      throw new Error('Query to register not found');
    }
    // Delegate to remote function which handles DBSP registration & persistence
    const res = await this.remote.query('fn::query::register($config)', {
      config: {
        ...queryState.config,
        clientId: this.clientId,
      },
    });
    console.log('xxxxxxx', res);
    return;
    this.logger.debug(
      { queryId: encodeRecordId(queryState.config.id), array },
      'createdRemoteQuery'
    );
    await this.dataModule.updateQueryRemoteArray(hash, array);
  }

  private async heartbeatQuery(queryId: string) {
    await this.remote.query('fn::query::heartbeat($id)', {
      id: queryId,
    });
  }

  private async cleanupQuery(queryId: string) {
    await this.remote.query(`DELETE $id`, {
      id: queryId,
    });
  }
}
