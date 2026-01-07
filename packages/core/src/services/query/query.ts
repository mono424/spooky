import { QueryHash, Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { Table, RecordId, Duration, Uuid } from 'surrealdb';
import { RemoteDatabaseService } from '../database/remote.js';
import { LocalDatabaseService } from '../database/local.js';
import { Incantation } from './incantation.js';
import { createLogger, Logger } from '../logger.js';
import {
  createQueryEventSystem,
  QueryEventSystem,
  QueryEventTypeMap,
  QueryEventTypes,
} from './events.js';
import { Event } from '../../events/index.js';
import { decodeFromSpooky, parseRecordIdString } from '../utils.js';
import { flattenIdTree } from '../sync/utils.js';
import { SchemaStructure, TableModel } from '@spooky/query-builder';

export class QueryManager<S extends SchemaStructure> {
  private activeQueries: Map<QueryHash, Incantation<any>> = new Map();
  private liveQueryUuid: string | null = null;
  private events: QueryEventSystem;
  private logger: Logger;

  public get eventsSystem() {
    return this.events;
  }

  constructor(
    private schema: S,
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private clientId: string | undefined, // undefined is valid for optional clientId, but argument position is fixed
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'QueryManager' });
    this.events = createQueryEventSystem();
    this.events.subscribe(
      QueryEventTypes.IncantationIncomingRemoteUpdate,
      this.handleIncomingRemoteUpdate.bind(this)
    );
  }

  public getQueriesThatInvolveTable(tableName: string) {
    return [...this.activeQueries.values()].filter((q) => q.invlovesTable(tableName));
  }

  public getActiveQueries() {
    return Array.from(this.activeQueries.values());
  }

  private async setClientId() {
    await this.remote.getClient().set('_spooky_client_id', this.clientId);
    this.logger.debug({ clientId: this.clientId }, 'ClientId set');
    // .query('LET $_spooky_client_id = $clientId', { clientId: this.clientId });
  }

  public async init() {
    await this.setClientId();
    await this.startLiveQuery();
  }

  private handleIncomingRemoteUpdate(
    event: Event<QueryEventTypeMap, 'QUERY_INCANTATION_INCOMING_REMOTE_UPDATE'>
  ) {
    const { incantationId, records, remoteHash, remoteTree } = event.payload;
    const incantation = this.activeQueries.get(incantationId.id.toString());
    if (!incantation) {
      return;
    }

    const remoteIds = new Set(flattenIdTree(remoteTree).map((node) => node.id));
    const validRecords: any[] = [];
    const orphanedRecords: any[] = [];

    for (const r of records) {
      const decoded = decodeFromSpooky(
        this.schema,
        incantation.tableName,
        r as unknown as TableModel<
          Extract<
            S['tables'][number],
            {
              name: string;
            }
          >
        >
      );

      const id = (decoded as any).id;
      const idStr = id instanceof RecordId ? id.toString() : id;

      if (idStr && remoteIds.has(idStr)) {
        validRecords.push(decoded);
      } else {
        orphanedRecords.push(decoded);
      }
    }

    this.logger.debug(
      {
        incantationId: incantationId.toString(),
        queryHash: incantationId.id.toString(),
        totalRecords: records.length,
        validRecords: validRecords.length,
        orphanedRecords: orphanedRecords.length,
      },
      'Handling incoming remote update'
    );

    incantation.updateLocalState(validRecords, remoteHash, remoteTree);
    this.events.emit(QueryEventTypes.IncantationUpdated, {
      incantationId,
      records: validRecords,
    });

    void this.verifyAndPurgeOrphans(orphanedRecords);
  }

  private async verifyAndPurgeOrphans(orphanedRecords: any[]) {
    if (orphanedRecords.length === 0) return;

    const idsToCheck = orphanedRecords
      .map((r) => r.id)
      .filter((id) => !!id)
      .map((id) => (id instanceof RecordId ? id : parseRecordIdString(id.toString())));

    if (idsToCheck.length === 0) return;

    this.logger.debug({ count: idsToCheck.length }, 'Verifying orphaned records against remote');

    try {
      const [existing] = await this.remote.query<[{ id: RecordId }[]]>('SELECT id FROM $ids', {
        ids: idsToCheck,
      });

      const existingIdsSet = new Set(existing.map((r) => r.id.toString()));
      const toDelete = idsToCheck.filter((id) => !existingIdsSet.has(id.toString()));

      if (toDelete.length > 0) {
        this.logger.info(
          { count: toDelete.length, ids: toDelete.map((id) => id.toString()) },
          'Purging confirmed orphaned records'
        );
        await this.local.query('DELETE $ids', { ids: toDelete });
      } else {
        this.logger.debug(
          { count: idsToCheck.length },
          'All orphaned records still exist remotely (ghost records)'
        );
      }
    } catch (err) {
      this.logger.error({ err }, 'Failed to verify/purge orphans');
    }
  }

  private async register(
    tableName: string,
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    involvedTables: string[] = []
  ): Promise<QueryHash> {
    const id = await this.calculateHash({
      clientId: this.clientId,
      surrealql,
      params,
    });

    const recordId = new RecordId('_spooky_incantation', id);

    // Helper for retrying DB operations (e.g. "Can not open transaction")
    const withRetry = async <T>(
      operation: () => Promise<T>,
      retries = 3,
      delayMs = 100
    ): Promise<T> => {
      let lastError;
      for (let i = 0; i < retries; i++) {
        try {
          return await operation();
        } catch (err: any) {
          lastError = err;
          // Check for transaction error or generic connection issues
          if (
            err?.message?.includes('Can not open transaction') ||
            err?.message?.includes('transaction')
          ) {
            this.logger.warn(
              {
                attempt: i + 1,
                retries,
                error: err.message,
              },
              'Retrying DB operation due to transaction error'
            );

            await new Promise((res) => setTimeout(res, delayMs * (i + 1))); // Linear backoff
            continue;
          }
          throw err;
        }
      }
      throw lastError;
    };

    let [existing] = await withRetry(() =>
      this.local.query<[IncantationData]>('SELECT * FROM ONLY $id', {
        id: recordId,
      })
    );

    if (!existing) {
      existing = await withRetry(() =>
        this.local
          .getClient()
          .create<IncantationData>(recordId)
          .content({
            id: recordId,
            surrealQL: surrealql,
            params: params,
            clientId: this.clientId,
            hash: id,
            tree: null,
            lastActiveAt: new Date(),
            ttl: new Duration(ttl),
            meta: {
              tableName,
              involvedTables,
            },
          })
      );
    }

    if (!existing) {
      throw new Error('Failed to create or retrieve incantation');
    }

    if (!this.activeQueries.has(id)) {
      const incantation = new Incantation({
        id: recordId,
        surrealql,
        params,
        hash: existing.hash,
        lastActiveAt: existing.lastActiveAt,
        ttl: existing.ttl,
        tree: existing.tree,
        meta: {
          tableName,
          involvedTables,
        },
      });
      this.activeQueries.set(id, incantation);
      await this.initLifecycle(incantation);
    }

    return id;
  }

  async query(
    tableName: string,
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    involvedTables: string[] = []
  ): Promise<QueryHash> {
    return this.register(tableName, surrealql, params, ttl, involvedTables);
  }

  private async initLifecycle(incantation: Incantation<any>) {
    this.events.emit(QueryEventTypes.IncantationInitialized, {
      incantationId: incantation.id,
      surrealql: incantation.surrealql,
      params: incantation.params ?? {},
      ttl: incantation.ttl,
    });

    await incantation.startTTLHeartbeat(() => {
      this.events.emit(QueryEventTypes.IncantationTTLHeartbeat, {
        incantationId: incantation.id,
      });
    });
  }

  subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options: { immediate?: boolean } = {}
  ): () => void {
    const id = this.events.subscribe(QueryEventTypes.IncantationUpdated, (event) => {
      const incomingId = event.payload.incantationId.id.toString();
      if (incomingId === queryHash) {
        this.logger.debug(
          {
            queryHash,
            recordCount: event.payload.records.length,
          },
          'Subscription callback triggered'
        );

        callback(event.payload.records);
      } else {
        // this.logger.trace({ incomingId, queryHash }, 'Subscription ignored mismatch');
      }
    });

    if (options.immediate) {
      const records = this.activeQueries.get(queryHash)?.records ?? [];
      callback(records);
    }

    return () => {
      this.events.unsubscribe(id);
    };
  }

  private async startLiveQuery() {
    this.logger.debug({ clientId: this.clientId }, 'Starting live query');
    const [queryUuid] = await this.remote.query<[Uuid]>(
      'LIVE SELECT * FROM _spooky_incantation WHERE clientId = $clientId',
      {
        clientId: this.clientId,
      }
    );

    (await this.remote.getClient().liveOf(queryUuid)).subscribe((message) => {
      this.logger.debug({ message }, 'Live update received');
      if (message.action === 'UPDATE' || message.action === 'CREATE') {
        const { id, hash, tree } = message.value;
        if (!(id instanceof RecordId) || !hash || !tree) {
          return;
        }

        const incantation = this.activeQueries.get(id.id.toString());
        if (!incantation) {
          this.logger.warn({ id: id.toString() }, 'Live update for unknown incantation');
          return;
        }

        this.events.emit(QueryEventTypes.IncantationRemoteHashUpdate, {
          incantationId: id as RecordId<string>,
          surrealql: incantation.surrealql,
          params: incantation.params ?? {},
          localHash: incantation.hash,
          localTree: incantation.tree,
          remoteHash: hash as string,
          remoteTree: tree as any,
        });
      }
    });
  }

  private async calculateHash(data: any): Promise<string> {
    const content = JSON.stringify(data);
    const msgBuffer = new TextEncoder().encode(content);
    const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
  }
}
