import { QueryHash, Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { Table, RecordId, Duration } from 'surrealdb';
import { RemoteDatabaseService } from '../database/remote.js';
import { LocalDatabaseService } from '../database/local.js';
import { Incantation } from './incantation.js';
import {
  createQueryEventSystem,
  QueryEventSystem,
  QueryEventTypeMap,
  QueryEventTypes,
} from './events.js';
import { Event } from '../../events/index.js';
import { decodeFromSpooky } from '../utils.js';
import { SchemaStructure, TableModel } from '@spooky/query-builder';

export class QueryManager<S extends SchemaStructure> {
  private activeQueries: Map<QueryHash, Incantation<any>> = new Map();
  private liveQueryUuid: string | null = null;
  private events: QueryEventSystem;

  public get eventsSystem() {
    return this.events;
  }

  constructor(
    private schema: S,
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private clientId: string
  ) {
    this.events = createQueryEventSystem();
    this.events.subscribe(
      QueryEventTypes.IncantationIncomingRemoteUpdate,
      this.handleIncomingRemoteUpdate.bind(this)
    );
  }

  public async init() {
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

    const outRecords = records.map((r) =>
      decodeFromSpooky(
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
      )
    );

    incantation.updateLocalState(outRecords, remoteHash, remoteTree);
    this.events.emit(QueryEventTypes.IncantationUpdated, {
      incantationId,
      records: outRecords,
    });
  }

  private async register(
    tableName: string,
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive = '10m'
  ): Promise<QueryHash> {
    const effectiveTtl = ttl || '10m';
    const id = await this.calculateHash({
      surrealql,
      params,
      clientId: this.clientId,
    });

    const recordId = new RecordId('_spooky_incantation', id);

    await this.local
      .getClient()
      .upsert<IncantationData>(recordId)
      .content({
        Id: id,
        SurrealQL: surrealql,
        Params: params,
        ClientId: this.clientId,
        Hash: id,
        Tree: null,
        LastActiveAt: new Date(),
        TTL: new Duration(effectiveTtl),
      });

    if (!this.activeQueries.has(id)) {
      const incantation = new Incantation({
        id: recordId,
        surrealql,
        params,
        hash: id,
        lastActiveAt: Date.now(),
        ttl: effectiveTtl,
        tree: null,
        meta: {
          tableName,
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
    ttl: QueryTimeToLive = '10m'
  ): Promise<QueryHash> {
    return this.register(tableName, surrealql, params, ttl);
  }

  private async initLifecycle(incantation: Incantation<any>) {
    this.events.emit(QueryEventTypes.IncantationInitialized, {
      incantationId: incantation.id,
      surrealql: incantation.surrealql,
      params: incantation.params,
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
      if (event.payload.incantationId.id.toString() === queryHash) {
        callback(event.payload.records);
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
    await this.remote
      .getClient()
      .query('LET $_spooky_client_id = $clientId', { clientId: this.clientId });
    const liveQuery = await this.remote.getClient().live(new Table('_spooky_incantation'));

    console.log('live queryUuid', liveQuery);
    await liveQuery.subscribe(async (message) => {
      console.log('live query message', message);
      const { action, recordId, value } = message;
      if (action === 'UPDATE' || action === 'CREATE') {
        const { Hash: hash, Tree: tree } = value;
        if (!hash || !tree) {
          return;
        }
        const incantation = this.activeQueries.get(recordId.id.toString());
        if (!incantation) {
          return;
        }

        console.log('live test', value);
        console.log('live hash', hash);
        console.log('live tree', tree);

        this.events.emit(QueryEventTypes.IncantationRemoteHashUpdate, {
          incantationId: recordId,
          surrealql: incantation.surrealql,
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
    const result = await (
      this.local.getClient().query('RETURN crypto::blake3($content)', { content }) as any
    ).collect();
    return result[0] as string;
  }
}
