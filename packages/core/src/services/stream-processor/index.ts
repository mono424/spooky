import init, { SpookyProcessor } from 'spooky-stream-processor-wasm';
import { EventDefinition, EventSystem } from '../../events/index.js';
import { Logger } from 'pino';
import { LocalDatabaseService } from '../database/index.js';
import { Incantation } from '../query/incantation.js';
import { WasmProcessor, WasmStreamUpdate } from './wasm-types.js';

// Define the shape of an update from the Wasm module
// Matches MaterializedViewUpdate struct
export interface StreamUpdate {
  query_id: string;
  localHash: string;
  localArray: any; // Flat array structure [[id, version], ...]
}

// Define events map
export type StreamProcessorEvents = {
  stream_update: EventDefinition<'stream_update', StreamUpdate[]>;
};

export class StreamProcessorService {
  private processor: WasmProcessor | undefined;
  private isInitialized = false;

  constructor(
    public events: EventSystem<StreamProcessorEvents>,
    private db: LocalDatabaseService,
    private logger: Logger
  ) {}

  /**
   * Initialize the WASM module and processor.
   * This must be called before using other methods.
   */
  async init() {
    if (this.isInitialized) return;

    this.logger.info('[StreamProcessor] Initializing WASM...');
    try {
      await init(); // Initialize the WASM module
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
                UPSERT
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
   */
  ingest(table: string, op: string, id: string, record: any): WasmStreamUpdate[] {
    this.logger.debug({ table, op, id }, '[StreamProcessor] Ingesting record');
    // DEEP LOGGING
    console.log('[StreamProcessor] ingest called', {
      table,
      op,
      id,
      record: JSON.stringify(record, (key, value) =>
        typeof value === 'bigint' ? value.toString() : value
      ),
    });

    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping ingest');
      return [];
    }

    try {
      const normalizedRecord = this.normalizeValue(record);
      // DEEP LOGGING (Normalized)
      console.log('[StreamProcessor] ingest normalized record', JSON.stringify(normalizedRecord));

      const rawUpdates = this.processor.ingest(table, op, id, normalizedRecord);
      console.log('[StreamProcessor] ingest result', rawUpdates);

      if (rawUpdates && Array.isArray(rawUpdates) && rawUpdates.length > 0) {
        const updates: StreamUpdate[] = rawUpdates.map((u: WasmStreamUpdate) => ({
          query_id: u.query_id,
          localHash: u.result_hash,
          localArray: u.result_data,
        }));
        this.events.emit('stream_update', updates);
      }
      this.saveState();
      return rawUpdates;
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error during ingestion');
      console.error('[StreamProcessor] Erroring during ingestion', e);
    }
    return [];
  }

  /**
   * Register a new incantation (query plan).
   * Emits 'stream_update' with the initial result.
   */
  registerIncantation(incantation: Incantation<unknown>) {
    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping registration');
      return;
    }

    this.logger.debug(
      {
        incantationId: incantation.id,
        surrealQL: incantation.surrealql,
        params: incantation.params,
      },
      '[StreamProcessor] Registering incantation'
    );
    // DEEP LOGGING
    console.log('[StreamProcessor] registerIncantation called', {
      id: incantation.id.toString(),
      sql: incantation.surrealql,
      params: incantation.params,
    });

    try {
      const normalizedParams = this.normalizeValue(incantation.params);
      console.log('[StreamProcessor] registerIncantation normalized params', normalizedParams);

      const initialUpdate = this.processor.register_view({
        id: incantation.id.toString(),
        surrealQL: incantation.surrealql,
        params: normalizedParams,
        clientId: 'local',
        ttl: incantation.ttl.toString(),
        lastActiveAt: new Date().toISOString(),
      });

      console.log('[StreamProcessor] register_view result', initialUpdate);
      console.log('[StreamProcessor] normalizedParams used:', JSON.stringify(normalizedParams));

      if (!initialUpdate) {
        throw new Error('Failed to register incantation');
      }
      const update: StreamUpdate = {
        query_id: initialUpdate.query_id,
        localHash: initialUpdate.result_hash,
        localArray: initialUpdate.result_data,
      };
      this.saveState();
      this.logger.debug(
        {
          incantationId: incantation.id,
          surrealQL: incantation.surrealql,
          params: incantation.params,
        },
        '[StreamProcessor] Registered incantation'
      );
      return update;
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error registering incantation');
      console.error('[StreamProcessor] Error registering incantation', e);
      throw e;
    }
  }

  /**
   * Unregister an incantation by ID.
   */
  unregisterIncantation(id: string) {
    if (!this.processor) return;
    try {
      this.processor.unregister_view(id);
      this.saveState();
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error unregistering incantation');
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
        console.log('[normalizeValue] RecordId detected, converted to:', result);
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
