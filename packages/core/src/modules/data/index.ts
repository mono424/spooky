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
  QueryConfigRecord,
} from '../../types.js';
import {
  parseRecordIdString,
  extractIdPart,
  encodeRecordId,
  parseDuration,
  withRetry,
  surql,
  parseParams,
  extractTablePart,
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
    this.logger.info({ Category: 'spooky-client::DataModule::init' }, 'DataModule initialized');
  }

  // ==================== QUERY MANAGEMENT ====================

  /**
   * Register a query and return its hash for subscriptions
   */
  async query<T extends TableNames<S>>(
    tableName: T,
    surqlString: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive
  ): Promise<QueryHash> {
    const hash = await this.calculateHash({ surql: surqlString, params });
    this.logger.debug(
      { hash, Category: 'spooky-client::DataModule::query' },
      'Query Initialization: started'
    );

    const recordId = new RecordId('_spooky_query', hash);

    if (this.activeQueries.has(hash)) {
      this.logger.debug(
        { hash, Category: 'spooky-client::DataModule::query' },
        'Query Initialization: exists, returning'
      );
      return hash;
    }

    this.logger.debug(
      { hash, Category: 'spooky-client::DataModule::query' },
      'Query Initialization: not found, creating new query'
    );
    const queryState = await this.createNewQuery<T>({
      recordId,
      surql: surqlString,
      params,
      ttl,
      tableName,
    });

    const { localArray } = this.cache.registerQuery({
      queryHash: hash,
      surql: surqlString,
      params,
      ttl: new Duration(ttl),
      lastActiveAt: new Date(),
    });

    await withRetry(this.logger, () =>
      this.local.query(surql.seal(surql.updateSet('id', ['localArray'])), {
        id: recordId,
        localArray,
      })
    );

    this.activeQueries.set(hash, queryState);
    this.startTTLHeartbeat(queryState);
    this.logger.debug(
      {
        hash,
        tableName,
        recordCount: queryState.records.length,
        Category: 'spooky-client::DataModule::query',
      },
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
      this.logger.warn(
        { queryHash, Category: 'spooky-client::DataModule::onStreamUpdate' },
        'Received update for unknown query. Skipping...'
      );
      return;
    }

    try {
      // Fetch updated records
      const [records] = await this.local.query<[Record<string, any>[]]>(
        queryState.config.surql,
        queryState.config.params
      );

      // Update state
      queryState.records = records || [];
      queryState.config.localArray = localArray;
      queryState.updateCount++;
      await this.local.query(surql.seal(surql.updateSet('id', ['localArray'])), {
        id: queryState.config.id,
        localArray,
      });

      // Notify subscribers
      const subscribers = this.subscriptions.get(queryHash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }

      this.logger.debug(
        {
          queryHash,
          recordCount: records?.length,
          Category: 'spooky-client::DataModule::onStreamUpdate',
        },
        'Query updated from stream'
      );
    } catch (err) {
      this.logger.error(
        { err, queryHash, Category: 'spooky-client::DataModule::onStreamUpdate' },
        'Failed to fetch records for stream update'
      );
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
      this.logger.warn(
        { id, Category: 'spooky-client::DataModule::updateQueryLocalArray' },
        'Query to update local array not found'
      );
      return;
    }
    queryState.config.localArray = localArray;
    await this.local.query(surql.seal(surql.updateSet('id', ['localArray'])), {
      id: queryState.config.id,
      localArray,
    });
  }

  async updateQueryRemoteArray(hash: string, remoteArray: RecordVersionArray): Promise<void> {
    const queryState = this.getQueryByHash(hash);
    if (!queryState) {
      this.logger.warn(
        { hash, Category: 'spooky-client::DataModule::updateQueryRemoteArray' },
        'Query to update remote array not found'
      );
      return;
    }
    queryState.config.remoteArray = remoteArray;
    await this.local.query(surql.seal(surql.updateSet('id', ['remoteArray'])), {
      id: queryState.config.id,
      remoteArray,
    });
  }

  // ==================== MUTATION MANAGEMENT ====================

  /**
   * Create a new record
   */
  async create<T extends Record<string, unknown>>(id: string, data: T): Promise<T> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const params = parseParams(tableSchema.columns, data);
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
        data: params,
        mid: mutationId,
      })
    );

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache (which handles DBSP ingestion)
    await this.cache.save(
      {
        table: tableName,
        op: 'CREATE',
        record: parsedRecord,
      },
      true,
      true
    );

    // Emit mutation event for sync
    const mutationEvent: MutationEvent = {
      type: 'create',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'spooky-client::DataModule::create' }, 'Record created');

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
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const params = parseParams(tableSchema.columns, data);
    const mutationId = parseRecordIdString(`_spooky_pending_mutations:${Date.now()}`);

    const query = surql.seal(
      surql.tx([
        surql.let('updated', surql.updateMerge('id', 'data')),
        surql.createMutation('update', 'mid', 'id', 'data'),
        surql.returnObject([{ key: 'target', variable: 'updated' }]),
      ])
    );

    const [{ target }] = await withRetry(this.logger, () =>
      this.local.query<[{ target: T }]>(query, {
        id: rid,
        mid: mutationId,
        data: params,
      })
    );

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache
    await this.cache.save(
      {
        table: table,
        op: 'UPDATE',
        record: parsedRecord,
      },
      true,
      true
    );

    // Emit mutation event
    const mutationEvent: MutationEvent = {
      type: 'update',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'spooky-client::DataModule::update' }, 'Record updated');

    return target;
  }

  /**
   * Delete a record
   */
  async delete(table: string, id: string): Promise<void> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const mutationId = parseRecordIdString(`_spooky_pending_mutations:${Date.now()}`);

    const query = surql.seal(
      surql.tx([surql.delete('id'), surql.createMutation('delete', 'mid', 'id')])
    );

    await withRetry(this.logger, () => this.local.query(query, { id: rid, mid: mutationId }));
    await this.cache.delete(table, id, true, true);

    // Emit mutation event
    const mutationEvent: MutationEvent = {
      type: 'delete',
      mutation_id: mutationId,
      record_id: rid,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'spooky-client::DataModule::delete' }, 'Record deleted');
  }

  // ==================== PRIVATE HELPERS ====================

  private async createNewQuery<T extends TableNames<S>>({
    recordId,
    surql: surqlString,
    params,
    ttl,
    tableName,
  }: {
    recordId: RecordId;
    surql: string;
    params: Record<string, any>;
    ttl: QueryTimeToLive;
    tableName: T;
  }): Promise<QueryState> {
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    let [configRecord] = await withRetry(this.logger, () =>
      this.local.query<[QueryConfigRecord]>('SELECT * FROM ONLY $id', {
        id: recordId,
      })
    );

    if (!configRecord) {
      const [createdRecord] = await withRetry(this.logger, () =>
        this.local.query<[QueryConfigRecord]>(surql.seal(surql.create('id', 'data')), {
          id: recordId,
          data: {
            surql: surqlString,
            params: params,
            localArray: [],
            remoteArray: [],
            lastActiveAt: new Date(),
            ttl,
            tableName,
          },
        })
      );
      configRecord = createdRecord;
    }

    const config: QueryConfig = {
      ...configRecord,
      id: recordId,
      params: parseParams(tableSchema.columns, configRecord.params),
    };

    let records: Record<string, any>[] = [];
    try {
      const [result] = await this.local.query<[Record<string, any>[]]>(surqlString, params);
      records = result || [];
    } catch (err) {
      this.logger.warn(
        { err, Category: 'spooky-client::DataModule::createNewQuery' },
        'Failed to load initial cached records'
      );
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
      this.logger.debug(
        {
          id: encodeRecordId(queryState.config.id),
          Category: 'spooky-client::DataModule::startTTLHeartbeat',
        },
        'TTL heartbeat'
      );
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
