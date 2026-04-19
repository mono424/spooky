import { RecordId, Duration } from 'surrealdb';
import type {
  SchemaStructure,
  TableNames,
  BackendNames,
  BackendRoutes,
  RoutePayload,
} from '@spooky-sync/query-builder';
import type { LocalDatabaseService } from '../../services/database/index';
import type { CacheModule, RecordWithId } from '../cache/index';
import type { Logger } from '../../services/logger/index';
import type { StreamUpdate } from '../../services/stream-processor/index';
import type {
  QueryConfig,
  QueryHash,
  QueryState,
  QueryTimeToLive,
  QueryUpdateCallback,
  MutationCallback,
  RecordVersionArray,
  QueryConfigRecord,
  UpdateOptions,
  RunOptions} from '../../types';
import {
  parseRecordIdString,
  extractIdPart,
  encodeRecordId,
  parseDuration,
  withRetry,
  surql,
  parseParams,
  extractTablePart,
  generateId,
} from '../../utils/index';
import type { CreateEvent, DeleteEvent, UpdateEvent } from '../sync/index';
import type { PushEventOptions } from '../../events/index';

/**
 * DataModule - Unified query and mutation management
 *
 * Merges the functionality of QueryManager and MutationManager.
 * Uses CacheModule for all storage operations.
 */
export class DataModule<S extends SchemaStructure> {
  private activeQueries: Map<QueryHash, QueryState> = new Map();
  private pendingQueries: Map<QueryHash, Promise<QueryHash>> = new Map();
  private subscriptions: Map<QueryHash, Set<QueryUpdateCallback>> = new Map();
  private mutationCallbacks: Set<MutationCallback> = new Set();
  private debounceTimers: Map<QueryHash, NodeJS.Timeout> = new Map();
  private logger: Logger;

  constructor(
    private cache: CacheModule,
    private local: LocalDatabaseService,
    private schema: S,
    logger: Logger,
    private streamDebounceTime: number = 100
  ) {
    this.logger = logger.child({ service: 'DataModule' });
  }

  async init(): Promise<void> {
    this.logger.info({ Category: 'sp00ky-client::DataModule::init' }, 'DataModule initialized');
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
      { hash, Category: 'sp00ky-client::DataModule::query' },
      'Query Initialization: started'
    );

    const recordId = new RecordId('_00_query', hash);

    if (this.activeQueries.has(hash)) {
      this.logger.debug(
        { hash, Category: 'sp00ky-client::DataModule::query' },
        'Query Initialization: exists, returning'
      );
      return hash;
    }

    // Another call is already creating this query — wait for it
    if (this.pendingQueries.has(hash)) {
      this.logger.debug(
        { hash, Category: 'sp00ky-client::DataModule::query' },
        'Query Initialization: pending, waiting for existing creation'
      );
      await this.pendingQueries.get(hash);
      return hash;
    }

    this.logger.debug(
      { hash, Category: 'sp00ky-client::DataModule::query' },
      'Query Initialization: not found, creating new query'
    );

    // Create the query and track the pending promise
    const promise = this.createAndRegisterQuery<T>(hash, recordId, surqlString, params, ttl, tableName);
    this.pendingQueries.set(hash, promise);
    try {
      await promise;
    } finally {
      this.pendingQueries.delete(hash);
    }

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
    const { queryHash, op } = update;

    // Only debounce UPDATE operations
    // CREATE and DELETE should propagate immediately
    if (op === 'UPDATE') {
      // Clear existing timer if any
      if (this.debounceTimers.has(queryHash)) {
        // oxlint-disable-next-line no-non-null-assertion -- guarded by .has() check above
        clearTimeout(this.debounceTimers.get(queryHash)!);
      }

      // Set new timer
      const timer = setTimeout(async () => {
        this.debounceTimers.delete(queryHash);
        await this.processStreamUpdate(update);
      }, this.streamDebounceTime);

      this.debounceTimers.set(queryHash, timer);
    } else {
      // CREATE and DELETE - process immediately
      await this.processStreamUpdate(update);
    }
  }

  private async processStreamUpdate(update: StreamUpdate): Promise<void> {
    const { queryHash, localArray } = update;
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) {
      this.logger.warn(
        { queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
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
      const newRecords = records || [];
      queryState.config.localArray = localArray;
      await this.local.query(surql.seal(surql.updateSet('id', ['localArray'])), {
        id: queryState.config.id,
        localArray,
      });

      // Skip notification if records haven't changed
      const prevJson = JSON.stringify(queryState.records);
      const newJson = JSON.stringify(newRecords);
      queryState.records = newRecords;
      if (prevJson === newJson) {
        this.logger.debug(
          { queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
          'Query records unchanged, skipping notification'
        );
        return;
      }

      queryState.updateCount++;

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
          Category: 'sp00ky-client::DataModule::onStreamUpdate',
        },
        'Query updated from stream'
      );
    } catch (err) {
      this.logger.error(
        { err, queryHash, Category: 'sp00ky-client::DataModule::onStreamUpdate' },
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
        { id, Category: 'sp00ky-client::DataModule::updateQueryLocalArray' },
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
        { hash, Category: 'sp00ky-client::DataModule::updateQueryRemoteArray' },
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

  /**
   * Called after a query's initial sync completes.
   * Ensures subscribers are notified even if no stream updates fired (e.g. empty result set).
   */
  async notifyQuerySynced(queryHash: string): Promise<void> {
    const queryState = this.activeQueries.get(queryHash);
    if (!queryState) return;

    // Re-query local DB for latest data
    const [records] = await this.local.query<[Record<string, any>[]]>(
      queryState.config.surql,
      queryState.config.params
    );
    const newRecords = records || [];
    const changed = JSON.stringify(queryState.records) !== JSON.stringify(newRecords);
    queryState.records = newRecords;

    // Notify if data changed OR if this is the first sync (updateCount === 0)
    // The latter handles "query truly has no results" so UI can stop loading
    if (changed || queryState.updateCount === 0) {
      queryState.updateCount++;
      const subscribers = this.subscriptions.get(queryHash);
      if (subscribers) {
        for (const callback of subscribers) {
          callback(queryState.records);
        }
      }
    }
  }

  // ====================      RUN JOBS       ====================

  async run<B extends BackendNames<S>, R extends BackendRoutes<S, B>>(
    backend: B,
    path: R,
    data: RoutePayload<S, B, R>,
    options?: RunOptions
  ): Promise<void> {
    const route = this.schema.backends?.[backend]?.routes?.[path];
    if (!route) {
      throw new Error(`Route ${backend}.${path} not found`);
    }

    const tableName = this.schema.backends?.[backend]?.outboxTable;
    if (!tableName) {
      throw new Error(`Outbox table for backend ${backend} not found`);
    }

    const payload: Record<string, unknown> = {};
    for (const argName of Object.keys(route.args)) {
      const arg = route.args[argName];
      if ((data as Record<string, unknown>)[argName] === undefined && arg.optional === false) {
        throw new Error(`Missing required argument ${argName}`);
      }
      payload[argName] = (data as Record<string, unknown>)[argName];
    }

    const record: Record<string, unknown> = {
      path,
      payload: JSON.stringify(payload),
      max_retries: options?.max_retries ?? 3,
      retry_strategy: options?.retry_strategy ?? 'linear',
    };

    if (options?.timeout != null) {
      record.timeout = options.timeout;
    }

    if (options?.assignedTo) {
      record.assigned_to = options.assignedTo;
    }

    const recordId = `${tableName}:${generateId()}`;
    await this.create(recordId, record);
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
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

    const dataKeys = Object.keys(params).map((key) => ({ key, variable: `data_${key}` }));
    const prefixedParams = Object.fromEntries(
      dataKeys.map(({ key, variable }) => [variable, params[key]])
    );
    const query = surql.seal<T>(
      surql.tx([
        surql.createSet('id', dataKeys),
        surql.createMutation('create', 'mid', 'id', 'data'),
      ]),
      { resultIndex: 0 }
    );

    const target = await withRetry(this.logger, () =>
      this.local.execute(query, {
        id: rid,
        mid: mutationId,
        ...prefixedParams,
      })
    );

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache (which handles DBSP ingestion)
    await this.cache.save(
      {
        table: tableName,
        op: 'CREATE',
        record: parsedRecord,
        version: 1,
      },
      true
    );

    // Emit mutation event for sync
    const mutationEvent: CreateEvent = {
      type: 'create',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
      tableName,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::create' }, 'Record created');

    return target;
  }

  /**
   * Update an existing record
   */
  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>,
    options?: UpdateOptions
  ): Promise<T> {
    const tableName = extractTablePart(id);
    const tableSchema = this.schema.tables.find((t) => t.name === tableName);
    if (!tableSchema) {
      throw new Error(`Table ${tableName} not found`);
    }

    const rid = parseRecordIdString(id);
    const params = parseParams(tableSchema.columns, data);
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

    // Note: CRDT state is pushed directly to the _00_crdt table by CrdtField.pushToRemote(),
    // NOT through the record update pipeline. This keeps the record data clean.

    // Capture current record state before mutation for rollback support
    const [beforeRecord] = await withRetry(this.logger, () =>
      this.local.query<[Record<string, any>]>('SELECT * FROM ONLY $id', { id: rid })
    );

    const query = surql.seal<{ target: T }>(
      surql.tx([
        surql.updateSet('id', [{ statement: '_00_rv += 1' }]),
        surql.let('updated', surql.updateMerge('id', 'data')),
        surql.createMutation('update', 'mid', 'id', 'data'),
        surql.returnObject([{ key: 'target', variable: 'updated' }]),
      ])
    );

    const { target } = await withRetry(this.logger, () =>
      this.local.execute(query, {
        id: rid,
        mid: mutationId,
        data: params,
      })
    );

    // Build a partial record with only the fields the user actually changed
    // This avoids overwriting rich relation objects (e.g. author: {id, name, ...})
    // with flat RecordIds from the UPDATE...MERGE result
    const updatedFields: Record<string, any> = { id: target.id };
    for (const key of Object.keys(data)) {
      if (key in target) {
        updatedFields[key] = (target as Record<string, any>)[key];
      }
    }
    if ('_00_rv' in (target as Record<string, any>)) {
      updatedFields._00_rv = (target as Record<string, any>)._00_rv;
    }
    this.replaceRecordInQueries(updatedFields);

    const parsedRecord = parseParams(tableSchema.columns, target) as RecordWithId;

    // Save to cache
    await this.cache.save(
      {
        table: table,
        op: 'UPDATE',
        record: parsedRecord,
        version: target._00_rv as number,
      },
      true
    );

    const pushEventOptions = parseUpdateOptions(id, data, options);

    // Emit mutation event
    const mutationEvent: UpdateEvent = {
      type: 'update',
      mutation_id: mutationId,
      record_id: rid,
      data: params,
      record: target,
      beforeRecord: beforeRecord || undefined,
      options: pushEventOptions,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::update' }, 'Record updated');

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
    const mutationId = parseRecordIdString(`_00_pending_mutations:${Date.now()}`);

    // Fetch the record before deleting so DBSP can match it against query predicates
    const [beforeRecords] = await this.local.query<[Record<string, any>[]]>(
      'SELECT * FROM ONLY $id',
      { id: rid }
    );
    const beforeRecord = beforeRecords ?? {};

    const query = surql.seal<void>(
      surql.tx([surql.delete('id'), surql.createMutation('delete', 'mid', 'id')])
    );

    await withRetry(this.logger, () => this.local.execute(query, { id: rid, mid: mutationId }));
    await this.cache.delete(table, id, true, beforeRecord);

    // DBSP may not emit view updates for DELETE ops —
    // manually notify all queries that reference this table
    for (const [queryHash, queryState] of this.activeQueries) {
      if (queryState.config.tableName === tableName) {
        await this.notifyQuerySynced(queryHash);
      }
    }

    // Emit mutation event
    const mutationEvent: DeleteEvent = {
      type: 'delete',
      mutation_id: mutationId,
      record_id: rid,
    };

    for (const callback of this.mutationCallbacks) {
      callback([mutationEvent]);
    }

    this.logger.debug({ id, Category: 'sp00ky-client::DataModule::delete' }, 'Record deleted');
  }

  // ==================== ROLLBACK METHODS ====================

  /**
   * Rollback a failed optimistic create by deleting the record locally
   */
  async rollbackCreate(recordId: RecordId, tableName: string): Promise<void> {
    const id = encodeRecordId(recordId);

    try {
      await withRetry(this.logger, () =>
        this.local.query('DELETE $id', { id: recordId })
      );
      await this.cache.delete(tableName, id, true);
      this.removeRecordFromQueries(recordId);

      this.logger.info(
        { id, tableName, Category: 'sp00ky-client::DataModule::rollbackCreate' },
        'Rolled back optimistic create'
      );
    } catch (err) {
      this.logger.error(
        { err, id, tableName, Category: 'sp00ky-client::DataModule::rollbackCreate' },
        'Failed to rollback create'
      );
    }
  }

  /**
   * Rollback a failed optimistic update by restoring the previous record state
   */
  async rollbackUpdate(
    recordId: RecordId,
    tableName: string,
    beforeRecord: Record<string, unknown>
  ): Promise<void> {
    const id = encodeRecordId(recordId);

    try {
      const { id: _recordId, ...content } = beforeRecord;
      await withRetry(this.logger, () =>
        this.local.query(surql.seal(surql.upsert('id', 'content')), {
          id: recordId,
          content,
        })
      );

      const tableSchema = this.schema.tables.find((t) => t.name === tableName);
      const parsedRecord = tableSchema
        ? (parseParams(tableSchema.columns, beforeRecord) as RecordWithId)
        : (beforeRecord as RecordWithId);

      await this.cache.save(
        {
          table: tableName,
          op: 'UPDATE',
          record: parsedRecord,
          version: (beforeRecord._00_rv as number) || 1,
        },
        true
      );

      // Replace in active queries for immediate UI update
      await this.replaceRecordInQueries(beforeRecord);

      this.logger.info(
        { id, tableName, Category: 'sp00ky-client::DataModule::rollbackUpdate' },
        'Rolled back optimistic update'
      );
    } catch (err) {
      this.logger.error(
        { err, id, tableName, Category: 'sp00ky-client::DataModule::rollbackUpdate' },
        'Failed to rollback update'
      );
    }
  }

  /**
   * Remove a record from all active query states and notify subscribers
   */
  private removeRecordFromQueries(recordId: RecordId): void {
    const encodedId = encodeRecordId(recordId);

    for (const [queryHash, queryState] of this.activeQueries.entries()) {
      const index = queryState.records.findIndex((r) => {
        const rId = r.id instanceof RecordId ? encodeRecordId(r.id) : String(r.id);
        return rId === encodedId;
      });

      if (index !== -1) {
        queryState.records.splice(index, 1);
        const subscribers = this.subscriptions.get(queryHash);
        if (subscribers) {
          for (const callback of subscribers) {
            callback(queryState.records);
          }
        }
      }
    }
  }

  // ==================== PRIVATE HELPERS ====================

  private async createAndRegisterQuery<T extends TableNames<S>>(
    hash: QueryHash,
    recordId: RecordId,
    surqlString: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    tableName: T
  ): Promise<QueryHash> {
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
        Category: 'sp00ky-client::DataModule::query',
      },
      'Query registered'
    );

    return hash;
  }

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
        { err, Category: 'sp00ky-client::DataModule::createNewQuery' },
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
          Category: 'sp00ky-client::DataModule::startTTLHeartbeat',
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

  private async replaceRecordInQueries(record: Record<string, any>): Promise<void> {
    for (const [queryHash, queryState] of this.activeQueries.entries()) {
      const index = queryState.records.findIndex((r) => r.id === record.id);
      if (index !== -1) {
        queryState.records[index] = { ...queryState.records[index], ...record };
        // Notify subscribers so UI updates immediately
        const subscribers = this.subscriptions.get(queryHash);
        if (subscribers) {
          for (const callback of subscribers) {
            callback(queryState.records);
          }
        }
      }
    }
  }
}

// ==================== HELPER FUNCTIONS ====================

/**
 * Parse update options to generate push event options
 */
export function parseUpdateOptions(
  id: string,
  data: any,
  options?: UpdateOptions
): PushEventOptions {
  let pushEventOptions: PushEventOptions = {};
  if (options?.debounced) {
    const delay = options.debounced !== true ? (options.debounced?.delay ?? 200) : 200;
    const keyType = options.debounced !== true ? (options.debounced?.key ?? id) : id;
    const key =
      keyType === 'recordId_x_fields' ? `${id}::${Object.keys(data).toSorted().join('#')}` : id;

    pushEventOptions = {
      debounced: {
        delay,
        key,
      },
    };
  }
  return pushEventOptions;
}
