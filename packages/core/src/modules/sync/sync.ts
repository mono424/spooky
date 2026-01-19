import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { RecordVersionArray } from '../../types.js';
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
import { ArraySyncer } from './utils.js';
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
    void this.startLiveQuery();
  }

  private async startLiveQuery() {
    this.logger.debug({ clientId: this.clientId }, 'Starting live query');
    // Ensure clientId is set in remote if needed, but SpookySync usually assumes auth is handled.
    // If we need to set the variable for the session:
    // await this.remote.getClient().query('LET $clientId = $id', { id: this.clientId });
    // Actually QueryManager did: await this.remote.getClient().set('_spooky_client_id', this.clientId);
    await this.remote.getClient().set('_spooky_client_id', this.clientId);

    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _spooky_incantation WHERE clientId = $clientId',
      {
        clientId: this.clientId,
      }
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug({ message }, 'Live update received');
      if (message.action === 'UPDATE' || message.action === 'CREATE') {
        const { id, hash, array, tree } = message.value;
        const remoteArray = array || tree; // Still called tree I think

        if (!(id instanceof RecordId) || !hash || !remoteArray) {
          return;
        }

        this.handleRemoteIncantationChange(
          id,
          hash as string,
          (remoteArray || []) as RecordVersionArray
        ).catch((err) => {
          this.logger.error({ err }, 'Error handling remote incantation change');
        });
      }
    });
  }

  private async handleRemoteIncantationChange(
    incantationId: RecordId,
    remoteHash: string,
    remoteArray: RecordVersionArray
  ) {
    // Fetch local state to get necessary params
    const existing = this.dataManager.getIncantation(incantationId);
    if (!existing) {
      this.logger.warn(
        { incantationId: incantationId.toString() },
        'Received remote update for unknown local incantation'
      );
      return;
    }

    const surrealql = existing.surrealql;
    const { params, localHash, localArray } = existing;

    await this.syncIncantation({
      incantationId,
      surrealql,
      localArray,
      localHash,
      remoteHash,
      remoteArray,
      params: params ?? {},
    });
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
        const { incantationId, surrealql, params, localArray, localHash, remoteHash, remoteArray } =
          event.payload;
        return this.syncIncantation({
          incantationId,
          surrealql,
          localArray,
          localHash,
          remoteHash,
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
    const {
      incantationId,
      surrealql,
      params,
      ttl,
      localHash: pLocalHash,
      localArray: pLocalArray,
    } = event.payload;

    const effectiveTtl = ttl || '10m';
    try {
      let existing = this.dataManager.getIncantation(incantationId);
      this.logger.debug({ existing }, 'Register Incantation state');

      // Use payload values as fallback if existing record doesn't have them
      // This is critical for preventing the "empty start" loop if the incantation was just initialized
      // with known state from the stream processor or previous context.
      // NOTE: We use || and length checks because ?? doesn't work with empty strings/arrays
      // (they are not null/undefined, so ?? returns them instead of the fallback)
      const localHash = existing?.localHash || pLocalHash || '';
      const localArray = existing?.localArray?.length ? existing.localArray : (pLocalArray ?? []);

      await this.updateLocalIncantation(
        incantationId,
        {
          surrealql,
          params,
          localHash,
          localArray,
        },
        {
          updateRecord: existing ? false : true,
        }
      );

      const { hash: remoteHash, array: remoteArray } = await this.createRemoteIncantation(
        incantationId,
        surrealql,
        params,
        effectiveTtl
      );

      await this.syncIncantation({
        incantationId,
        surrealql,
        localArray,
        localHash,
        remoteHash,
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
    const [{ hash, array }] = await this.remote.query<
      [{ hash: string; array: RecordVersionArray }]
    >('fn::incantation::register($config)', {
      config: safeConfig,
    });

    this.logger.debug(
      { incantationId: incantationId.toString(), hash, array },
      'createdRemoteIncantation'
    );
    return { hash, array };
  }

  private async syncIncantation({
    incantationId,
    surrealql,
    localArray,
    localHash,
    remoteHash,
    remoteArray,
    params,
  }: {
    incantationId: RecordId<string>;
    surrealql: string;
    localArray: RecordVersionArray;
    localHash: string;
    remoteHash: string;
    remoteArray: RecordVersionArray;
    params: Record<string, any>;
  }) {
    this.logger.debug(
      {
        incantationId: incantationId.toString(),
        localHash,
        remoteHash,
        localArray,
        remoteArray,
        params,
      },
      'syncIncantation'
    );

    const isDifferent = localHash !== remoteHash;
    if (!isDifferent) {
      return;
    }

    const arraySyncer = new ArraySyncer(localArray, remoteArray);
    let maxIter = 10;
    while (maxIter > 0) {
      const { added, updated, removed } = await this.syncEngine.syncRecords(
        arraySyncer,
        incantationId,
        remoteArray
      );

      if (added.length === 0 && updated.length === 0 && removed.length === 0) {
        break;
      }
      this.logger.debug({ added, updated, removed }, '[SpookySync] syncIncantation iteration');
      maxIter--;
      if (maxIter <= 0) {
        this.logger.warn(
          { incantationId: incantationId.toString() },
          'syncIncantation maxIter reached'
        );
      }
    }

    await this.updateLocalIncantation(
      incantationId,
      {
        surrealql,
        params,
        localHash: remoteHash, // After sync, local should match remote
        localArray: remoteArray, // After sync, local should match remote
        remoteHash,
        remoteArray,
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
      localHash,
      localArray,
      remoteHash,
      remoteArray,
    }: {
      surrealql: string;
      params?: Record<string, any>;
      localHash?: string;
      localArray?: RecordVersionArray;
      remoteHash?: string;
      remoteArray?: RecordVersionArray;
    },
    {
      updateRecord = true,
    }: {
      updateRecord?: boolean;
    }
  ) {
    if (updateRecord) {
      const content: any = {};
      if (localHash !== undefined) content.localHash = localHash;
      if (localArray !== undefined) content.localArray = localArray;
      if (remoteHash !== undefined) content.remoteHash = remoteHash;
      if (remoteArray !== undefined) content.remoteArray = remoteArray;

      await this.updateIncantationRecord(incantationId, content);
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
        localHash,
        localArray,
        remoteHash,
        remoteArray,
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
