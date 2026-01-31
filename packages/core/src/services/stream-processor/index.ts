import init, { SpookyProcessor } from '@spooky/ssp-wasm';
import { EventDefinition, EventSystem } from '../../events/index.js';
import { Logger } from 'pino';
import { LocalDatabaseService } from '../database/index.js';
import { WasmProcessor, WasmStreamUpdate } from './wasm-types.js';
import { Duration } from 'surrealdb';
import { PersistenceClient, QueryTimeToLive, RecordVersionArray } from '../../types.js';

// Simple interface for query plan registration (replaces Incantation class)
interface QueryPlanConfig {
  queryHash: string;
  surql: string;
  params: Record<string, any>;
  ttl: QueryTimeToLive | Duration;
  lastActiveAt: Date;
  localArray: RecordVersionArray;
  remoteArray: RecordVersionArray;
  meta: {
    tableName: string;
    involvedTables?: string[];
  };
}

// Define the shape of an update from the Wasm module
// Matches MaterializedViewUpdate struct
export interface StreamUpdate {
  queryHash: string;
  localArray: RecordVersionArray;
}

// Define events map (kept for DevTools compatibility)
export type StreamProcessorEvents = {
  stream_update: EventDefinition<'stream_update', StreamUpdate[]>;
};
/**
 * Interface for receiving stream updates directly.
 * Implemented by DataManager and DevToolsService for direct coupling.
 */
export interface StreamUpdateReceiver {
  onStreamUpdate(update: StreamUpdate): void;
}

export class StreamProcessorService {
  private logger: Logger;
  private processor: WasmProcessor | undefined;
  private isInitialized = false;
  private receivers: StreamUpdateReceiver[] = [];

  constructor(
    public events: EventSystem<StreamProcessorEvents>,
    private db: LocalDatabaseService,
    private persistenceClient: PersistenceClient,
    logger: Logger
  ) {
    this.logger = logger.child({ name: 'StreamProcessorService' });
  }

  /**
   * Add a receiver for stream updates.
   * Multiple receivers can be registered (DataManager, DevTools, etc.)
   */
  addReceiver(receiver: StreamUpdateReceiver) {
    this.receivers.push(receiver);
  }

  private notifyUpdates(updates: StreamUpdate[]) {
    for (const update of updates) {
      for (const receiver of this.receivers) {
        receiver.onStreamUpdate(update);
      }
    }
  }

  /**
   * Initialize the WASM module and processor.
   * This must be called before using other methods.
   */
  async init() {
    if (this.isInitialized) return;

    this.logger.info(
      { Category: 'spooky-client:SPS::init' },
      'Initializing WASM...'
    );
    try {
      await init(); // Initialize the WASM module (web target)
      // We cast the generated SpookyProcessor to our interface which is safer
      this.processor = new SpookyProcessor() as unknown as WasmProcessor;

      // Try to load state
      await this.loadState();

      this.isInitialized = true;
      this.logger.info(
        { Category: 'spooky-client:SPS::init' },
        'Initialized successfully'
      );
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::init' },
        'Failed to initialize'
      );
      throw e;
    }
  }

  async loadState() {
    if (!this.processor) return;
    try {
      const result = await this.persistenceClient.get('_spooky_stream_processor_state');

      // Check if we have a valid result from the query
      if (
        Array.isArray(result) &&
        result.length > 0 &&
        Array.isArray(result[0]) &&
        result[0].length > 0 &&
        result[0][0]?.state
      ) {
        const state = result[0][0].state;
        this.logger.info(
          {
            stateLength: state.length,
            Category: 'spooky-client:SPS::loadState',
          },
          'Loading state from DB'
        );
        // Assuming processor has a load_state method matching the save_state behavior
        // If not, we might need to adjust based on the actual WASM API
        if (typeof (this.processor as any).load_state === 'function') {
          (this.processor as any).load_state(state);
        } else {
          this.logger.warn(
            { Category: 'spooky-client:SPS::loadState' },
            'load_state method not found on processor'
          );
        }
      } else {
        this.logger.info(
          { Category: 'spooky-client:SPS::loadState' },
          'No saved state found'
        );
      }
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::loadState' },
        'Failed to load state'
      );
    }
  }

  async saveState() {
    if (!this.processor) return;
    try {
      // Assuming processor has a save_state method that returns the state string/bytes
      if (typeof (this.processor as any).save_state === 'function') {
        const state = (this.processor as any).save_state();
        if (state) {
          await this.persistenceClient.set('_spooky_stream_processor_state', state);
          this.logger.trace(
            { Category: 'spooky-client:SPS::saveState' },
            'State saved'
          );
        }
      }
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::saveState' },
        'Failed to save state'
      );
    }
  }

  /**
   * Ingest a record change into the processor.
   * Emits 'stream_update' event if materialized views are affected.
   * @param isOptimistic true = local mutation (increment versions), false = remote sync (keep versions)
   */
  /**
  /**
   * Ingest a record change into the processor.
   * Emits 'stream_update' event if materialized views are affected.
   */
  ingest(
    table: string,
    op: string,
    id: string,
    record: any
  ): WasmStreamUpdate[] {
    this.logger.debug(
      {
        table,
        op,
        id,
        Category: 'spooky-client:SPS::ingest',
      },
      'Ingesting record'
    );

    if (!this.processor) {
      this.logger.warn(
        { Category: 'spooky-client:SPS::ingest' },
        'Not initialized, skipping ingest'
      );
      return [];
    }

    try {
      const normalizedRecord = this.normalizeValue(record);

      const rawUpdates = this.processor.ingest_single(table, op, id, normalizedRecord);
      this.logger.debug(
        { rawUpdates, Category: 'spooky-client:SPS::ingest' },
        'ingest result'
      );

      if (rawUpdates && Array.isArray(rawUpdates) && rawUpdates.length > 0) {
        const updates: StreamUpdate[] = rawUpdates.map((u: WasmStreamUpdate) => ({
          queryHash: u.query_id,
          localArray: u.result_data,
        }));
        // Direct handler call instead of event
        this.notifyUpdates(updates);
      }
      this.saveState();
      return rawUpdates;
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::ingest' },
        'Error during ingestion'
      );
    }
    return [];
  }

  /**
   * Ingest multiple record changes in a single batch.
   * More efficient than calling ingest() multiple times as it:
   * 1. Processes all records together in WASM
   * 2. Emits a SINGLE stream_update event with all results
   * 3. Saves state only once at the end
   */
  ingestBatch(
    batch: Array<{ table: string; op: string; record: any; version?: number }>
  ): WasmStreamUpdate[] {
    if (batch.length === 0) return [];

    this.logger.debug(
      {
        batchSize: batch.length,
        Category: 'spooky-client:SPS::ingestBatch',
      },
      'Ingesting batch'
    );

    if (!this.processor) {
      this.logger.warn(
        { Category: 'spooky-client:SPS::ingestBatch' },
        'Not initialized, skipping batch ingest'
      );
      return [];
    }

    try {
      const allUpdates: WasmStreamUpdate[] = [];

      // Manually iterate and ingest single records
      for (const item of batch) {
        const normalizedRecord = this.normalizeValue(item.record);
        // We use ingest_single for each item
        const updates = this.processor.ingest_single(
          item.table,
          item.op,
          item.record.id, // Assuming record.id is available as in previous code
          normalizedRecord
        );
        
        if (updates && Array.isArray(updates)) {
            allUpdates.push(...updates);
        }
      }

      this.logger.debug(
        {
          updateCount: allUpdates.length,
          Category: 'spooky-client:SPS::ingestBatch',
        },
        'batch ingest result'
      );

      if (allUpdates.length > 0) {
        const updates: StreamUpdate[] = allUpdates.map((u: WasmStreamUpdate) => ({
          queryHash: u.query_id,
          localArray: u.result_data,
        }));
        // Direct handler call instead of event
        this.notifyUpdates(updates);
      }
      this.saveState();
      return allUpdates;
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::ingestBatch' },
        'Error during batch ingestion'
      );
    }
    return [];
  }

  /**
   * Register a new query plan.
   * Emits 'stream_update' with the initial result.
   */
  registerQueryPlan(queryPlan: QueryPlanConfig) {
    if (!this.processor) {
      this.logger.warn(
        { Category: 'spooky-client:SPS::registerQueryPlan' },
        'Not initialized, skipping registration'
      );
      return;
    }

    this.logger.debug(
      {
        queryHash: queryPlan.queryHash,
        surql: queryPlan.surql,
        params: queryPlan.params,
        Category: 'spooky-client:SPS::registerQueryPlan',
      },
      'Registering query plan'
    );

    try {
      const normalizedParams = this.normalizeValue(queryPlan.params);

      const initialUpdate = this.processor.register_view({
        id: queryPlan.queryHash,
        surql: queryPlan.surql,
        params: normalizedParams,
        clientId: 'local',
        ttl: queryPlan.ttl.toString(),
        lastActiveAt: new Date().toISOString(),
      });

      this.logger.debug(
        { initialUpdate, Category: 'spooky-client:SPS::registerQueryPlan' },
        'register_view result'
      );

      if (!initialUpdate) {
        throw new Error('Failed to register query plan');
      }
      const update: StreamUpdate = {
        queryHash: initialUpdate.query_id,
        localArray: initialUpdate.result_data,
      };
      this.saveState();
      this.logger.debug(
        {
          queryHash: queryPlan.queryHash,
          surql: queryPlan.surql,
          params: queryPlan.params,
          Category: 'spooky-client:SPS::registerQueryPlan',
        },
        'Registered query plan'
      );
      return update;
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::registerQueryPlan' },
        'Error registering query plan'
      );
      throw e;
    }
  }

  /**
   * Unregister a query plan by ID.
   */
  unregisterQueryPlan(queryHash: string) {
    if (!this.processor) return;
    try {
      this.processor.unregister_view(queryHash);
      this.saveState();
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'spooky-client:SPS::unregisterQueryPlan' },
        'Error unregistering query plan'
      );
    }
  }

  private normalizeValue(value: any): any {
    if (value === null || value === undefined) return value;

    if (typeof value === 'object') {
      // RecordId detection using duck typing (constructor.name may be minified)
      // SurrealDB's RecordId has: table (getter returning Table), id, and toString()
      // Check for table getter that has its own toString AND id property
      const hasTable = 'table' in value && typeof value.table?.toString === 'function';
      const hasId = 'id' in value;
      const hasToString = typeof value.toString === 'function';
      const isNotPlainObject = value.constructor !== Object;

      if (hasTable && hasId && hasToString && isNotPlainObject) {
        const result = value.toString();
        this.logger.trace(
          { result, Category: 'spooky-client:SPS::normalizeValue' },
          'RecordId detected'
        );
        return result;
      }

      // Fallback: old check for objects with tb and id (some internal representations)
      if ('tb' in value && 'id' in value && !('table' in value)) {
        return `${value.tb}:${value.id}`;
      }

      // Handle arrays recursively
      if (Array.isArray(value)) {
        return value.map((v) => this.normalizeValue(v));
      }

      // Handle plain objects recursively
      if (value.constructor === Object) {
        const out: any = {};
        for (const k in value) {
          out[k] = this.normalizeValue(value[k]);
        }
        return out;
      }
    }
    return value;
  }
}
