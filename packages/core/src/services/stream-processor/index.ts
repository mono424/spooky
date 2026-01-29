import init, { SpookyProcessor } from '@spooky/ssp-wasm';
import { EventDefinition, EventSystem } from '../../events/index.js';
import { Logger } from 'pino';
import { LocalDatabaseService } from '../database/index.js';
import { WasmProcessor, WasmStreamUpdate } from './wasm-types.js';
import { RecordId, Duration } from 'surrealdb';
import { QueryTimeToLive, RecordVersionArray } from '../../types.js';
import { encodeRecordId } from '../../utils/index.js';

// Simple interface for query plan registration (replaces Incantation class)
interface QueryPlanConfig {
  id: RecordId<string>;
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
  localArray: any; // Flat array structure [[id, version], ...]
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
  private processor: WasmProcessor | undefined;
  private isInitialized = false;
  private receivers: StreamUpdateReceiver[] = [];

  constructor(
    public events: EventSystem<StreamProcessorEvents>,
    private db: LocalDatabaseService,
    private logger: Logger
  ) {}

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

    this.logger.info('[StreamProcessor] Initializing WASM...');
    try {
      await init(); // Initialize the WASM module (web target)
      // We cast the generated SpookyProcessor to our interface which is safer
      this.processor = new SpookyProcessor() as unknown as WasmProcessor;

      // Try to load state
      await this.loadState();

      this.isInitialized = true;
      this.logger.info('[StreamProcessor] Initialized successfully');
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Failed to initialize');
      throw e;
    }
  }

  async loadState() {
    if (!this.processor) return;
    try {
      const result = await this.db.query<[{ state: string }[]]>(
        'SELECT state FROM _spooky_stream_processor_state LIMIT 1'
      );

      // Check if we have a valid result from the query
      if (
        Array.isArray(result) &&
        result.length > 0 &&
        Array.isArray(result[0]) &&
        result[0].length > 0 &&
        result[0][0]?.state
      ) {
        const state = result[0][0].state;
        this.logger.info({ stateLength: state.length }, '[StreamProcessor] Loading state from DB');
        // Assuming processor has a load_state method matching the save_state behavior
        // If not, we might need to adjust based on the actual WASM API
        if (typeof (this.processor as any).load_state === 'function') {
          (this.processor as any).load_state(state);
        } else {
          this.logger.warn('[StreamProcessor] load_state method not found on processor');
        }
      } else {
        this.logger.info('[StreamProcessor] No saved state found');
      }
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Failed to load state');
    }
  }

  async saveState() {
    if (!this.processor) return;
    try {
      // Assuming processor has a save_state method that returns the state string/bytes
      if (typeof (this.processor as any).save_state === 'function') {
        const state = (this.processor as any).save_state();
        if (state) {
          await this.db.query(
            `
                UPDATE _spooky_stream_processor_state 
                SET state = $state, updated_at = time::now() 
                WHERE id = 'singleton'
                `,
            { state }
          );
          this.logger.trace('[StreamProcessor] State saved');
        }
      }
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Failed to save state');
    }
  }

  /**
   * Ingest a record change into the processor.
   * Emits 'stream_update' event if materialized views are affected.
   * @param isOptimistic true = local mutation (increment versions), false = remote sync (keep versions)
   */
  ingest(
    table: string,
    op: string,
    id: string,
    record: any,
    isOptimistic: boolean = true
  ): WasmStreamUpdate[] {
    this.logger.debug({ table, op, id, isOptimistic }, '[StreamProcessor] Ingesting record');
    this.logger.debug(
      {
        table,
        op,
        id,
        record: JSON.stringify(record, (key, value) =>
          typeof value === 'bigint' ? value.toString() : value
        ),
      },
      '[StreamProcessor] ingest called'
    );

    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping ingest');
      return [];
    }

    try {
      const normalizedRecord = this.normalizeValue(record);
      this.logger.debug(
        { normalizedRecord: JSON.stringify(normalizedRecord) },
        '[StreamProcessor] ingest normalized record'
      );

      const rawUpdates = this.processor.ingest(table, op, id, normalizedRecord, isOptimistic);
      this.logger.debug({ rawUpdates }, '[StreamProcessor] ingest result');

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
      this.logger.error(e, '[StreamProcessor] Error during ingestion');
      this.logger.error({ error: e }, '[StreamProcessor] Erroring during ingestion');
    }
    return [];
  }

  /**
   * Ingest multiple record changes in a single batch.
   * More efficient than calling ingest() multiple times as it:
   * 1. Processes all records together in WASM
   * 2. Emits a SINGLE stream_update event with all results
   * 3. Saves state only once at the end
   * @param isOptimistic true = local mutation (increment versions), false = remote sync (keep versions)
   */
  ingestBatch(
    batch: Array<{ table: string; op: string; record: any; version?: number }>,
    isOptimistic: boolean = true
  ): WasmStreamUpdate[] {
    if (batch.length === 0) return [];

    this.logger.debug(
      { batchSize: batch.length, isOptimistic },
      '[StreamProcessor] Ingesting batch'
    );

    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping batch ingest');
      return [];
    }

    try {
      // Normalize all records in the batch
      const normalizedBatch = batch.map((item) => ({
        table: item.table,
        op: item.op,
        id: item.record.id,
        record: this.normalizeValue(item.record),
        version: item.version,
      }));

      const rawUpdates = this.processor.ingest_batch(normalizedBatch, isOptimistic);
      this.logger.debug(
        { updateCount: rawUpdates?.length },
        '[StreamProcessor] batch ingest result'
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
      this.logger.error(e, '[StreamProcessor] Error during batch ingestion');
    }
    return [];
  }

  /**
   * Explicitly set the version of a record in a specific view.
   * This is used during remote sync to ensure local state matches remote versioning.
   */
  setRecordVersion(queryId: string, recordId: string, version: number) {
    if (!this.processor) return;

    this.logger.debug({ queryId, recordId, version }, '[StreamProcessor] setRecordVersion');

    try {
      // Note: recordId must be fully qualified table:id string
      const update = this.processor.set_record_version(queryId, recordId, version);

      if (update) {
        const updates: StreamUpdate[] = [
          {
            queryHash: update.query_id,
            localArray: update.result_data,
          },
        ];
        // Direct handler call instead of event
        this.notifyUpdates(updates);
        this.saveState();
      }
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error setting record version');
    }
  }

  /**
   * Register a new query plan.
   * Emits 'stream_update' with the initial result.
   */
  registerQueryPlan(queryPlan: QueryPlanConfig) {
    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping registration');
      return;
    }

    this.logger.debug(
      {
        queryId: queryPlan.id,
        surql: queryPlan.surql,
        params: queryPlan.params,
      },
      '[StreamProcessor] Registering query plan'
    );
    this.logger.debug(
      {
        id: encodeRecordId(queryPlan.id),
        surql: queryPlan.surql,
        params: queryPlan.params,
      },
      '[StreamProcessor] registerQueryPlan called'
    );

    try {
      const normalizedParams = this.normalizeValue(queryPlan.params);
      this.logger.debug(
        { normalizedParams },
        '[StreamProcessor] registerQueryPlan normalized params'
      );

      const initialUpdate = this.processor.register_view({
        id: encodeRecordId(queryPlan.id),
        surql: queryPlan.surql,
        params: normalizedParams,
        clientId: 'local',
        ttl: queryPlan.ttl.toString(),
        lastActiveAt: new Date().toISOString(),
      });

      this.logger.debug({ initialUpdate }, '[StreamProcessor] register_view result');
      this.logger.debug(
        { normalizedParams: JSON.stringify(normalizedParams) },
        '[StreamProcessor] normalizedParams used'
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
          queryId: queryPlan.id,
          surql: queryPlan.surql,
          params: queryPlan.params,
        },
        '[StreamProcessor] Registered query plan'
      );
      return update;
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error registering query plan');
      this.logger.error({ error: e }, '[StreamProcessor] Error registering query plan');
      throw e;
    }
  }

  /**
   * Unregister a query plan by ID.
   */
  unregisterQueryPlan(id: string) {
    if (!this.processor) return;
    try {
      this.processor.unregister_view(id);
      this.saveState();
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error unregistering query plan');
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
        this.logger.trace({ result }, '[StreamProcessor] normalizeValue RecordId detected');
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
