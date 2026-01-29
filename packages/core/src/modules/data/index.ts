import { RecordId, Duration } from 'surrealdb';
import { SchemaStructure, TableNames } from '@spooky/query-builder';
import { LocalDatabaseService } from '../../services/database/index.js';
import { CacheModule, RecordWithId } from '../cache/index.js';
import { Logger } from '../../services/logger/index.js';
import { StreamUpdate } from '../../services/stream-processor/index.js';
import {
  MutationEvent,
  QueryConfig,
  QueryHash,
  QueryState,
  QueryTimeToLive,
  QueryUpdateCallback,
  MutationCallback,
  RecordVersionArray,
} from '../../types.js';
import {
  parseRecordIdString,
  encodeToSpooky,
  encodeToSpookyContent,
  extractIdPart,
  encodeRecordId,
  parseDuration,
  withRetry,
  surql,
} from '../../utils/index.js';

/**
 * DataModule - Unified query and mutation management
 *
 * Merges the functionality of QueryManager and MutationManager.
 * Uses CacheModule for all storage operations.
 */
export class DataModule<S extends SchemaStructure> {
  private activeQueries: Map<QueryHash, QueryState> = new Map();
  private subscriptions: Map<QueryHash, Set<QueryUpdateCallback>> = new Map();
  private mutationCallbacks: Set<MutationCallback> = new Set();
  private logger: Logger;

  constructor(
    private cache: CacheModule,
    private local: LocalDatabaseService,
    private schema: S,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'DataModule' });
  }

  async init(): Promise<void> {
    this.logger.info('DataModule initialized');
  }

  // ==================== QUERY MANAGEMENT ====================

  /**
   * Register a query and return its hash for subscriptions
   */
  async query<T extends TableNames<S>>(
    tableName: T,
    sql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive
  ): Promise<QueryHash> {
    const hash = await this.calculateHash({ sql, params });

    const recordId = new RecordId('_spooky_query', hash);

    if (this.activeQueries.has(hash)) {
      return hash;
    }

    const queryState = await this.createNewQuery<T>({
      recordId,
      sql,
      params,
      ttl,
      tableName,
    });

    const { localArray } = this.cache.registerQuery({
      id: recordId,
      sql,
      params,
      ttl: new Duration(ttl),
      lastActiveAt: new Date(),
    });

    await withRetry(this.logger, () =>
      this.local.getClient().upsert(recordId).replace({ localArray })
    );

    this.activeQueries.set(hash, queryState);
    this.startTTLHeartbeat(queryState);
    this.logger.debug(
      { hash, tableName, recordCount: queryState.records.length },
      'Query registered'
    );

    return hash;
  }

  /**
   * Subscribe to query updates
   */
  subscribe(
    queryHash: string,
    callback: QueryUpdateCallback,
    options: { immediate?: boolean } = {}
  ): () => void {
    if (!this.subscriptions.has(queryHash)) {
      this.subscriptions.set(queryHash, new Set());
    }

    this.subscriptions.get(queryHash)?.add(callback);

    if (options.immediate) {
      const query = this.activeQueries.get(queryHash);
      if (query) {
        callback(query.records);
      }
    }

    // Return unsubscribe function
    return () => {
      const subs = this.subscriptions.get(queryHash);
      if (subs) {
        subs.delete(callback);
        if (subs.size === 0) {
          this.subscriptions.delete(queryHash);
        }
      }
    };
  }

  /**
   * Subscribe to mutations (for sync)
   */
  onMutation(callback: MutationCallback): () => void {
    this.mutationCallbacks.add(callback);
    return () => {
      this.mutationCallbacks.delete(callback);
    };
  }

  /**
   * Handle stream updates from DBSP (via CacheModule)
   */
  async onStreamUpdate(update: StreamUpdate): Promise<void> {
    const { queryHash, localArray } = update;

    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) {
      this.logger.warn({ queryHash }, 'Received update for unknown query. Skipping...');
      return;
    }

    try {
      // Fetch updated records
      const [records] = await this.local.query<[Record<string, any>[]]>(
        queryState.config.sql,
        queryState.config.params
      );

      // Update state
      queryState.records = records || [];
      queryState.config.localArray = localArray;
      queryState.updateCount++;
      await this.local.getClient().upsert(queryState.config.id).replace({ localArray });

      // Notify subscribers
      const subscribers = this.subscriptions.get(queryHash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }

      this.logger.debug({ queryHash, recordCount: records?.length }, 'Query updated from stream');
    } catch (err) {
      this.logger.error({ err, queryHash }, 'Failed to fetch records for stream update');
    }
  }

  /**
   * Get query state (for sync and devtools)
   */
  getQueryByHash(hash: string): QueryState | undefined {
    return this.activeQueries.get(hash);
  }

  /**
   * Get query state by id (for sync and devtools)
   */
  getQueryById(id: RecordId<string>): QueryState | undefined {
    return this.activeQueries.get(extractIdPart(id));
  }

  /**
   * Get all active queries (for devtools)
   */
  getActiveQueries(): QueryState[] {
    return Array.from(this.activeQueries.values());
  }

  async updateQueryLocalArray(id: string, localArray: RecordVersionArray): Promise<void> {
    const queryState = this.activeQueries.get(id);
    if (!queryState) {
      this.logger.warn({ id }, 'Query to update local array not found');
      return;
    }
    queryState.config.localArray = localArray;
    await this.local.getClient().upsert(queryState.config.id).replace({ localArray });
  }

  async updateQueryRemoteArray(hash: string, remoteArray: RecordVersionArray): Promise<void> {
    const queryState = this.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn({ hash }, 'Query to update remote array not found');
      return;
    }
    queryState.config.remoteArray = remoteArray;
    await this.local.getClient().upsert(queryState.config.id).replace({ remoteArray });
  }

  // ==================== MUTATION MANAGEMENT ====================

  /**
   * Create a new record
   */
  async create<T extends Record<string, unknown>>(id: string, data: T): Promise<T> {
    const rid = parseRecordIdString(id);
    const tableName = rid.table.toString();
    const encodedData = encodeToSpookyContent(this.schema, tableName, data as any);

    const mutationId = parseRecordIdString(`_spooky_pending_mutations:${Date.now()}`);

    const query = surql.seal(
      surql.tx([
        surql.let('created', surql.create('id', 'data')),
        surql.createMutation('create', 'mid', 'id', 'data'),
        surql.returnObject([{ key: 'target', variable: 'created' }]),
      ])
    );

    const [{ target }] = await withRetry(this.logger, () =>
      this.local.query<[{ target: T }]>(query, {
        id: rid,
        data: encodedData,
        mid: mutationId,
      })
    );

    if (!target) {
      throw new Error('Failed to create record or mutation log');
    }

    if (!target.id) {
      throw Error(`Failed to gather record ID: ${id}`);
    }

    // Save to cache (which handles DBSP ingestion)
    await this.cache.save(
      {
        table: tableName,
        op: 'CREATE',
        record: target as RecordWithId,
      },
      true
    );

    // Emit mutation event for sync
    const mutationEvent: MutationEvent = {
      type: 'create',
      mutation_id: mutationId,
      record_id: rid,
      data: encodedData,
      record: target,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id }, 'Record created');

    return target;
  }

  /**
   * Update an existing record
   */
  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>
  ): Promise<T> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);
    const encodedData = encodeToSpookyContent(this.schema, table as any, data as any);

    const mutationId = parseRecordIdString(`_spooky_pending_mutations:${Date.now()}`);

    let target: T;

    const query = surql.seal(
      surql.tx([
        surql.let('updated', surql.updateMerge('id', 'data')),
        surql.let('mutation', surql.createMutation('update', 'mid', 'id', 'data')),
        surql.returnObject([{ key: 'target', variable: 'updated' }]),
      ])
    );

    const [result] = await withRetry(this.logger, () =>
      this.local.query<[{ target: T }]>(query, {
        id: rid,
        data: encodedData,
        mid: mutationId,
      })
    );

    target = result?.target!;
    if (!target) {
      throw new Error(`Failed to update record: ${id}`);
    }

    if (!target.id) {
      throw Error(`Failed to gather record ID: ${id}`);
    }

    // Save to cache
    await this.cache.save(
      {
        table: table,
        op: 'UPDATE',
        record: target as RecordWithId,
      },
      true
    );

    // Emit mutation event
    const mutationEvent: MutationEvent = {
      type: 'update',
      mutation_id: mutationId,
      record_id: rid,
      data: encodedData,
      record: target,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id }, 'Record updated');

    return target;
  }

  /**
   * Delete a record
   */
  async delete(table: string, id: string): Promise<void> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);

    const mutationId = parseRecordIdString(`_spooky_pending_mutations:${Date.now()}`);

    const query = surql.seal(
      surql.tx([surql.delete('id'), surql.createMutation('delete', 'mid', 'id')])
    );

    await withRetry(this.logger, () => this.local.query(query, { id: rid, mid: mutationId }));
    await this.cache.delete(table, encodeRecordId(rid), true);

    // Emit mutation event
    const mutationEvent: MutationEvent = {
      type: 'delete',
      mutation_id: mutationId,
      record_id: rid,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id }, 'Record deleted');
  }

  // ==================== PRIVATE HELPERS ====================

  private async createNewQuery<T extends TableNames<S>>({
    recordId,
    sql,
    params,
    ttl,
    tableName,
  }: {
    recordId: RecordId;
    sql: string;
    params: Record<string, any>;
    ttl: QueryTimeToLive;
    tableName: T;
  }): Promise<QueryState> {
    let [config] = await withRetry(this.logger, () =>
      this.local.query<[QueryConfig]>('SELECT * FROM ONLY $id', {
        id: recordId,
      })
    );

    if (!config) {
      config = await withRetry(this.logger, () =>
        this.local.getClient().create<QueryConfig>(recordId).content({
          id: recordId,
          sql: sql,
          params: params,
          localArray: [],
          remoteArray: [],
          lastActiveAt: new Date(),
          ttl,
          tableName,
        })
      );
    }

    if (!config) {
      throw new Error('Failed to create or retrieve query');
    }

    let records: Record<string, any>[] = [];
    try {
      const [result] = await this.local.query<[Record<string, any>[]]>(sql, params);
      records = result || [];
    } catch (err) {
      this.logger.warn({ err }, 'Failed to load initial cached records');
    }

    return {
      config,
      records,
      ttlTimer: null,
      ttlDurationMs: parseDuration(ttl),
      updateCount: 0,
    };
  }

  private async calculateHash(data: any): Promise<string> {
    const content = JSON.stringify(data);
    const msgBuffer = new TextEncoder().encode(content);
    const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
  }

  private startTTLHeartbeat(queryState: QueryState): void {
    if (queryState.ttlTimer) return;

    const heartbeatTime = Math.floor(queryState.ttlDurationMs * 0.9);

    queryState.ttlTimer = setTimeout(() => {
      // TODO: Emit heartbeat event for sync
      this.logger.debug({ id: encodeRecordId(queryState.config.id) }, 'TTL heartbeat');
      this.startTTLHeartbeat(queryState);
    }, heartbeatTime);
  }

  private stopTTLHeartbeat(queryState: QueryState): void {
    if (queryState.ttlTimer) {
      clearTimeout(queryState.ttlTimer);
      queryState.ttlTimer = null;
    }
  }
}
