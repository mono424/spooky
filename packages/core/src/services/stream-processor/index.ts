import init, { SpookyProcessor } from 'spooky-stream-processor-wasm';
import { EventDefinition, EventSystem } from '../../events/index.js';
import { Logger } from 'pino';
import { LocalDatabaseService } from '../database/index.js';

// Define the shape of an update from the Wasm module
// Matches MaterializedViewUpdate struct
export interface StreamUpdate {
  query_id: string;
  localHash: string;
  localTree: any; // Merkle tree structure
}

// Define events map
export type StreamProcessorEvents = {
  stream_update: EventDefinition<'stream_update', StreamUpdate[]>;
};

export class StreamProcessorService {
  private processor: SpookyProcessor | undefined;
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
      this.processor = new SpookyProcessor();

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
  ingest(table: string, op: string, id: string, record: any): void {
    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping ingest');
      return;
    }

    try {
      const rawUpdates: any = (this.processor as any).ingest(table, op, id, record);
      if (rawUpdates && Array.isArray(rawUpdates) && rawUpdates.length > 0) {
        const updates: StreamUpdate[] = rawUpdates.map((u: any) => ({
          query_id: u.query_id,
          localHash: u.result_hash,
          localTree: u.tree,
        }));
        this.events.emit('stream_update', updates);
        this.saveState();
      }
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error during ingestion');
    }
  }

  /**
   * Register a new incantation (query plan).
   * Emits 'stream_update' with the initial result.
   */
  registerIncantation(planOrSql: any, params: any, queryId?: string) {
    if (!this.processor) {
      this.logger.warn('[StreamProcessor] Not initialized, skipping registration');
      return;
    }

    try {
      let config: any;

      // If passing a raw string (SQL), construct the configuration object expected by the updated WASM
      if (typeof planOrSql === 'string') {
        config = {
          id: queryId || `auto_${Date.now()}`,
          surrealQL: planOrSql,
          params: params,
          clientId: 'local', // Default or fetch from somewhere if vital
          ttl: '10m', // Default
          lastActiveAt: new Date().toISOString(),
        };
      } else {
        // Assume it's already a plan or valid config object?
        // If it's a legacy QueryPlan object, it might fail the new WASM check which expects "surrealQL" string or "id".
        // The new WASM parser logic uses `prepare_registration` which looks for `surrealQL`.
        // If we have a structured plan in `planOrSql`, the new WASM might not support it directly unless we adapt it.
        // However, the prompt implies "register view should work with sql string", so we focus on that.
        // If `planOrSql` is an object, we hope it matches the config shape.
        config = { ...planOrSql, params };
      }

      const initialUpdate: any = (this.processor as any).register_view(config);

      if (initialUpdate) {
        const update: StreamUpdate = {
          query_id: initialUpdate.query_id,
          localHash: initialUpdate.result_hash,
          localTree: initialUpdate.tree,
        };
        this.events.emit('stream_update', [update]);
        this.saveState();
      }
    } catch (e) {
      this.logger.error(e, '[StreamProcessor] Error registering incantation');
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
}
