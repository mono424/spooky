import { QueryManager } from './query.js';
import { MutationManager } from './mutation.js';
import { QueryTimeToLive, QueryHash } from '../../types.js';
import { LocalDatabaseService } from '../../services/database/index.js';
import { StreamProcessorService } from '../../services/stream-processor/index.js';
import { SchemaStructure } from '@spooky/query-builder';
import { Logger } from '../../services/logger/index.js';
import { QueryEventSystem } from './events/query.js';
import { MutationEventSystem } from './events/mutation.js';

/**
 * DataManager - Unified facade for all local data operations.
 * Combines Query and Mutation functionality in a single API.
 */
export class DataManager<S extends SchemaStructure> {
  private queryManager: QueryManager<S>;
  private mutationManager: MutationManager<S>;

  constructor(
    schema: S,
    local: LocalDatabaseService,
    streamProcessor: StreamProcessorService,
    clientId: string | undefined,
    logger: Logger
  ) {
    this.queryManager = new QueryManager(schema, local, streamProcessor, clientId, logger);
    this.mutationManager = new MutationManager(schema, local, streamProcessor, logger);
  }

  // ============ Query API ============

  get queryEvents(): QueryEventSystem {
    return this.queryManager.eventsSystem;
  }

  get mutationEvents(): MutationEventSystem {
    return this.mutationManager.events;
  }

  async init(): Promise<void> {
    await this.queryManager.init();
  }

  /**
   * Register a query and get its hash for subscriptions
   */
  async query(
    tableName: string,
    surrealql: string,
    params: Record<string, any>,
    ttl: QueryTimeToLive,
    involvedTables: string[] = []
  ): Promise<QueryHash> {
    return this.queryManager.query(tableName, surrealql, params, ttl, involvedTables);
  }

  /**
   * Subscribe to query updates
   */
  subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options: { immediate?: boolean } = {}
  ): () => void {
    return this.queryManager.subscribe(queryHash, callback, options);
  }

  /**
   * Get active queries for DevTools
   */
  getActiveQueries() {
    return this.queryManager.getActiveQueries();
  }

  /**
   * Handle stream processor updates
   */
  handleStreamUpdate(update: any) {
    return this.queryManager.handleStreamUpdate(update);
  }

  /**
   * Handle incoming remote updates
   */
  handleIncomingUpdate(payload: any) {
    return this.queryManager.handleIncomingUpdate(payload);
  }

  // ============ Mutation API ============

  /**
   * Create a new record
   */
  async create<T extends Record<string, any>>(
    id: string,
    data: T,
    options?: { localOnly?: boolean }
  ): Promise<T> {
    return this.mutationManager.create(id, data, options);
  }

  /**
   * Update an existing record
   */
  async update<T extends Record<string, any>>(
    table: string,
    id: string,
    data: Partial<T>,
    options?: { localOnly?: boolean }
  ): Promise<T> {
    return this.mutationManager.update(table, id, data, options);
  }

  /**
   * Delete a record
   */
  async delete(table: string, id: string, options?: { localOnly?: boolean }): Promise<void> {
    return this.mutationManager.delete(table, id, options);
  }
}
