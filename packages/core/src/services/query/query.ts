import { QueryHash, Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { Table, RecordId, Duration, Uuid } from 'surrealdb';
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
    private clientId?: string
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

    console.log('[QueryManager] handleIncomingRemoteUpdate', {
      incantationId: incantationId.toString(),
      queryHash: incantationId.id.toString(),
      recordCount: records.length,
    });

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
    ttl: QueryTimeToLive
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
            console.warn(
              `[QueryManager] Retry ${i + 1}/${retries} due to transaction error:`,
              err.message
            );
            await new Promise((res) => setTimeout(res, delayMs * (i + 1))); // Linear backoff
            continue;
          }
          throw err;
        }
      }
      throw lastError;
    };

    let existing = await withRetry(() => this.local.getClient().select<IncantationData>(recordId));

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
          })
      );
    }

    if (!existing) {
      throw new Error('Failed to create incantation');
    }

    console.log('existing', existing);

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
    ttl: QueryTimeToLive
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
      const incomingId = event.payload.incantationId.id.toString();
      if (incomingId === queryHash) {
        console.log('[QueryManager] Subscription callback triggered', {
          queryHash,
          recordCount: event.payload.records.length,
        });
        callback(event.payload.records);
      } else {
        // console.log('[QueryManager] Subscription ignored mismatch', { incomingId, queryHash });
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
    // const queryUuid = await this.remote.getClient().live(new Table('_spooky_incantation')).diff();
    const [queryUuid] = await this.remote
      .getClient()
      .query('LIVE SELECT * FROM _spooky_incantation WHERE clientId = $clientId', {
        clientId: this.clientId,
      })
      .collect<[Uuid]>();

    console.log('XXLIVE QUERY REGISTER', queryUuid, this.clientId);
    await this.remote.subscribeLive(queryUuid.toString(), async (action, result) => {
      console.log('XXLIVE QUERY', action, result);
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
    const content = JSON.stringify(data);

    // Use Web Crypto API if available (Browser)
    if (typeof crypto !== 'undefined' && crypto.subtle) {
      const msgBuffer = new TextEncoder().encode(content);
      const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
      const hashArray = Array.from(new Uint8Array(hashBuffer));
      return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
    }

    // Fallback for Node.js (if applicable in this environment)
    // Assuming we are in a browser-like environment primarily due to 'use-query.ts' usage.
    // If strict Node support is needed, we'd import 'crypto'.
    // But for now, let's fall back to a simple non-crypto hash or try to keep the DB call?
    // No, DB call is broken.

    // Simplest fallback for now:
    console.warn('[QueryManager] crypto.subtle not found, using DB fallback (may fail)');
    const result = await (
      this.local.getClient().query('RETURN crypto::blake3($content)', { content }) as any
    ).collect();
    return result[0] as string;
  }
}
