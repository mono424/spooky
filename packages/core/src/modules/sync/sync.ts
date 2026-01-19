import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { createSyncEventSystem, SyncEventTypes } from './events/index.js';
import { Logger } from '../../services/logger/index.js';
import {
  CleanupEvent,
  DownEvent,
  DownQueue,
  HeartbeatEvent,
  RegisterEvent,
  UpEvent,
  UpQueue,
} from './queue/index.js';
import { RecordId, Duration, Uuid } from 'surrealdb';
import { applyRecordVersionDiff, ArraySyncer, createDiffFromDbOp } from './utils.js';
import { SyncEngine } from './engine.js';
import { SyncScheduler } from './scheduler.js';
import { SchemaStructure } from '@spooky/query-builder';
import { StreamProcessorService } from '../../services/stream-processor/index.js';
import { DataManager } from '../data/index.js';

export class SpookySync<S extends SchemaStructure> {
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
    private streamProcessor: StreamProcessorService,
    private dataManager: DataManager<S>,
    private clientId: string,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'SpookySync' });
    this.upQueue = new UpQueue(this.local);
    this.downQueue = new DownQueue(this.local);
    this.syncEngine = new SyncEngine(this.local, this.remote, this.streamProcessor, this.logger);
    this.scheduler = new SyncScheduler(
      this.upQueue,
      this.downQueue,
      this.processUpEvent.bind(this),
      this.processDownEvent.bind(this),
      this.logger
    );
  }

  public async init() {
    if (this.isInit) throw new Error('SpookySync is already initialized');
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
      console.log('__TEST__', message);
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
    incantationId: RecordId,
    recordId: RecordId,
    version: number
  ) {
    const existing = this.dataManager.getIncantation(incantationId);
    if (!existing) {
      this.logger.warn(
        { incantationId: incantationId.toString() },
        'Received remote update for unknown local incantation'
      );
      return;
    }

    const surrealql = existing.surrealql;
    const { params } = existing;

    const diff = createDiffFromDbOp(action, recordId, version);
    await this.syncIncantationDiff({ incantationId, surrealql, params: params ?? {} }, diff);
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
        return this.registerIncantation(event);
      case 'sync':
        const { incantationId, surrealql, params, localArray, remoteArray } = event.payload;
        return this.syncIncantationFromFullArray({
          incantationId,
          surrealql,
          localArray,
          remoteArray,
          params,
        });
      case 'heartbeat':
        return this.heartbeatIncantation(event);
      case 'cleanup':
        return this.cleanupIncantation(event);
    }
  }

  public async enqueueMutation(mutations: any[]) {
    this.scheduler.enqueueMutation(mutations);
  }

  private async registerIncantation(event: RegisterEvent) {
    const { incantationId, surrealql, params, ttl } = event.payload;

    const effectiveTtl = ttl || '10m';
    try {
      let existing = this.dataManager.getIncantation(incantationId);
      this.logger.debug({ existing }, 'Register Incantation state');

      await this.updateLocalIncantation(
        incantationId,
        {
          surrealql,
          params,
        },
        {
          updateRecord: existing ? false : true,
        }
      );

      const { array: remoteArray } = await this.createRemoteIncantation(
        incantationId,
        surrealql,
        params,
        effectiveTtl
      );

      await this.syncIncantationFromFullArray({
        incantationId,
        surrealql,
        localArray: existing?.localArray || [],
        remoteArray,
        params,
      });
    } catch (e) {
      this.logger.error({ err: e }, 'registerIncantation error');
      throw e;
    }
  }

  private async createRemoteIncantation(
    incantationId: RecordId<string>,
    surrealql: string,
    params: any,
    ttl: string | Duration
  ) {
    const config = {
      id: incantationId.id,
      surrealQL: surrealql,
      params,
      ttl: typeof ttl === 'string' ? new Duration(ttl) : ttl,
      lastActiveAt: new Date(),
      clientId: this.clientId,
      format: 'streaming',
    };

    const { ttl: _, ...safeConfig } = config;

    // Delegate to remote function which handles DBSP registration & persistence
    const [{ array }] = await this.remote.query<[{ array: RecordVersionArray }]>(
      'fn::incantation::register($config)',
      {
        config: safeConfig,
      }
    );

    this.logger.debug(
      { incantationId: incantationId.toString(), array },
      'createdRemoteIncantation'
    );
    return { array };
  }

  private async syncIncantationFromFullArray({
    incantationId,
    surrealql,
    localArray,
    remoteArray,
    params,
  }: {
    incantationId: RecordId<string>;
    surrealql: string;
    localArray: RecordVersionArray;
    remoteArray: RecordVersionArray;
    params: Record<string, any>;
  }) {
    this.logger.debug(
      {
        incantationId: incantationId.toString(),
        localArray,
        remoteArray,
        params,
      },
      'syncIncantation'
    );

    const diff = new ArraySyncer(localArray, remoteArray).nextSet();
    if (!diff) {
      return;
    }

    return this.syncIncantationDiff(
      {
        incantationId,
        surrealql,
        params,
      },
      diff
    );
  }

  private async syncIncantationDiff(
    {
      incantationId,
      surrealql,
      params,
    }: {
      incantationId: RecordId<string>;
      surrealql: string;
      params: Record<string, any>;
    },
    diff: RecordVersionDiff
  ) {
    const finalDiff = await this.syncEngine.syncRecords(diff);

    if (
      finalDiff.added.length === 0 &&
      finalDiff.updated.length === 0 &&
      finalDiff.removed.length === 0
    ) {
      return;
    }

    await this.updateLocalIncantation(
      incantationId,
      {
        surrealql,
        params,
        diff: finalDiff,
      },
      {
        updateRecord: true,
      }
    );
  }

  private async updateLocalIncantation(
    incantationId: RecordId<string>,
    {
      surrealql,
      params,
      diff,
    }: {
      surrealql: string;
      params?: Record<string, any>;
      diff?: RecordVersionDiff;
    },
    {
      updateRecord = true,
    }: {
      updateRecord?: boolean;
    }
  ) {
    const localVersions = this.dataManager.getIncantation(incantationId)?.localArray || [];
    const newVersions = diff ? applyRecordVersionDiff(localVersions, diff) : localVersions;

    if (updateRecord) {
      await this.updateIncantationRecord(incantationId, {
        localArray: newVersions,
      });
    }

    try {
      this.logger.debug(
        {
          incantationId: incantationId.toString(),
          surrealql,
          params,
        },
        'updateLocalIncantation Loading cached results start'
      );

      const [cachedResults] = await this.local.query<[Record<string, any>[]]>(surrealql, params);

      // Verify Orphans if we have a remote tree to check against
      // if (remoteTree) {
      //   void this.verifyAndPurgeOrphans(cachedResults, remoteTree);
      // }

      this.logger.debug(
        {
          incantationId: incantationId.toString(),
          recordCount: cachedResults?.length,
        },
        'updateLocalIncantation Loading cached results done'
      );

      this.events.emit(SyncEventTypes.IncantationUpdated, {
        incantationId,
        localArray: newVersions,
        remoteArray: newVersions,
        records: cachedResults || [],
      });
    } catch (e) {
      this.logger.error(
        { err: e },
        'updateLocalIncantation failed to query local db or emit event'
      );
    }
  }

  private async updateIncantationRecord(
    incantationId: RecordId<string>,
    content: Record<string, any>
  ) {
    try {
      this.logger.debug(
        { incantationId: incantationId.toString(), content },
        'Updating local incantation'
      );
      await this.local.query(`UPDATE $id MERGE $content`, {
        id: incantationId,
        content,
      });
    } catch (e) {
      this.logger.error({ err: e }, 'Failed to update local incantation record');
      throw e;
    }
  }

  private async heartbeatIncantation(event: HeartbeatEvent) {
    await this.remote.query('fn::incantation::heartbeat($id)', {
      id: event.payload.incantationId,
    });
  }

  private async cleanupIncantation(event: CleanupEvent) {
    await this.remote.query(`DELETE $id`, {
      id: event.payload.incantationId,
    });
  }
}
