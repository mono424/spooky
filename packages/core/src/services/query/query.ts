import { QueryHash, Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { Table, RecordId } from 'surrealdb';
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

export class QueryManager {
  private activeQueries: Map<QueryHash, Incantation<any>> = new Map();
  private liveQueryUuid: string | null = null;
  private events: QueryEventSystem;

  public get eventsSystem() {
    return this.events;
  }

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private clientId?: string
  ) {
    this.events = createQueryEventSystem();
    this.events.subscribe(
      QueryEventTypes.IncantationIncomingRemoteUpdate,
      this.handleIncomingRemoteUpdate.bind(this)
    );
    this.startLiveQuery();
  }

  private handleIncomingRemoteUpdate(
    event: Event<QueryEventTypeMap, 'QUERY_INCANTATION_INCOMING_REMOTE_UPDATE'>
  ) {
    const { incantationId, records, remoteHash, remoteTree } = event.payload;
    const incantation = this.activeQueries.get(incantationId.id.toString());
    if (!incantation) {
      return;
    }
    incantation.updateLocalState(records, remoteHash, remoteTree);
  }

  async register(
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive
  ): Promise<QueryHash> {
    const id = await this.calculateHash({
      surrealql,
      params,
    });

    const [incantationData] = await this.local
      .getClient()
      .query(
        `
      UPSERT _spooky_incantation:$id CONTENT {
        id: $id,
        surrealql: $surrealql,
        lastActiveAt: $lastActiveAt,
        ttl: $ttl,
        tree: $tree
      };
    `,
        {
          id,
          surrealql,
          lastActiveAt: new Date(),
          ttl,
          tree: null,
        }
      )
      .collect<IncantationData[]>();

    const incantationId = incantationData.id.id.toString();

    if (!this.activeQueries.has(incantationId)) {
      const incantation = new Incantation(incantationData);
      this.activeQueries.set(incantationId, incantation);
      await this.initLifecycle(incantation);
    }

    return incantationId;
  }

  async queryAdHoc(
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive
  ): Promise<QueryHash> {
    return this.register(surrealql, params, ttl);
  }

  private async initLifecycle(incantation: Incantation<any>) {
    this.events.emit(QueryEventTypes.IncantationInitialized, {
      incantationId: incantation.id,
      surrealql: incantation.surrealql,
      ttl: incantation.ttl,
    });

    await incantation.startTTLHeartbeat(() => {
      this.events.emit(QueryEventTypes.IncantationTTLHeartbeat, {
        incantationId: incantation.id,
      });
    });
  }

  private async startLiveQuery() {
    const queryUuid = await this.remote.getClient().live(new Table('_spooky_incantation')).diff();

    await this.remote.subscribeLive(queryUuid.toString(), async (action, result) => {
      if (action === 'UPDATE' || action === 'CREATE') {
        const { id, hash, tree } = result;
        if (!(id instanceof RecordId) || !hash || !tree) {
          return;
        }

        const incantation = this.activeQueries.get(id.id.toString());
        if (!incantation) {
          return;
        }

        this.events.emit(QueryEventTypes.IncantationRemoteHashUpdate, {
          incantationId: id as RecordId<string>,
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
    const result = await (
      this.local.getClient().query('RETURN crypto::blake3($data)', { data }) as any
    ).collect();
    return result[0] as string;
  }
}
