// oxlint-disable-next-line no-named-as-default -- WASM module default export convention
import init, { Sp00kyProcessor } from '@spooky-sync/ssp-wasm';
import type { EventDefinition, EventSystem } from '../../events/index';
import type { Logger } from 'pino';
import type { LocalDatabaseService } from '../database/index';
import type { WasmProcessor, WasmStreamUpdate } from './wasm-types';
import type { Duration } from 'surrealdb';
import type { PersistenceClient, QueryTimeToLive, RecordVersionArray } from '../../types';

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
  op?: 'CREATE' | 'UPDATE' | 'DELETE'; // Operation type for conditional debouncing
  /**
   * End-to-end ingest latency for the WASM call that produced this update,
   * in milliseconds. Populated by StreamProcessorService.ingest. Undefined
   * for the initial register_view snapshot.
   */
  materializationTimeMs?: number;
  /** SSP internal sub-phase timings (ms) for this ingest, from the WASM binding. */
  storeApplyMs?: number;
  circuitStepMs?: number;
  transformMs?: number;
  /**
   * One-shot registration timings (ms). Only set on the StreamUpdate returned
   * by `registerQueryPlan` (the register_view snapshot), not on ingest updates.
   */
  registration?: { parseMs: number; planMs: number; snapshotMs: number };
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
  // When true, `notifyUpdates` coalesces updates into `batchBuffer` (keyed by
  // queryHash) instead of dispatching them. Used to collapse the per-record
  // stream updates produced by a batched ingest into a single notification per
  // query, so the UI updates once after the whole batch rather than row-by-row.
  private batching = false;
  private batchBuffer: Map<string, StreamUpdate> = new Map();

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
    if (this.batching) {
      // Coalesce by queryHash instead of dispatching. The WASM `result_data`
      // (localArray) is the full materialized array, so last-write-wins
      // already reflects every prior ingest in the batch. We sum the
      // materialization times so the single recorded sample reflects the
      // batch's total work, and emit `op: 'CREATE'` on flush so the coalesced
      // update takes DataModule's immediate (non-debounced) path.
      for (const update of updates) {
        const prev = this.batchBuffer.get(update.queryHash);
        const sum = (a?: number, b?: number) => (a ?? 0) + (b ?? 0);
        this.batchBuffer.set(update.queryHash, {
          ...update,
          op: 'CREATE',
          materializationTimeMs: sum(prev?.materializationTimeMs, update.materializationTimeMs),
          storeApplyMs: sum(prev?.storeApplyMs, update.storeApplyMs),
          circuitStepMs: sum(prev?.circuitStepMs, update.circuitStepMs),
          transformMs: sum(prev?.transformMs, update.transformMs),
        });
      }
      return;
    }

    this.dispatchUpdates(updates);
  }

  private dispatchUpdates(updates: StreamUpdate[]) {
    for (const update of updates) {
      for (const receiver of this.receivers) {
        receiver.onStreamUpdate(update);
      }
    }
  }

  /**
   * Ingest a batch of record changes as a single bulk operation, firing only
   * one coalesced `StreamUpdate` per affected query once every record has been
   * ingested (instead of one update per record). Use this whenever multiple
   * records land at once — e.g. sync fetching N missing rows — so a list query
   * re-runs and the UI re-renders once for the whole batch rather than
   * row-by-row.
   *
   * Internally opens a coalescing window, ingests each record, then flushes;
   * processor state is persisted once for the whole batch. No-op for an empty
   * batch.
   */
  ingestMany(
    records: Array<{
      table: string;
      op: 'CREATE' | 'UPDATE' | 'DELETE';
      id: string;
      record: any;
    }>
  ): void {
    if (records.length === 0) return;

    this.beginCoalescing();
    try {
      for (const record of records) {
        this.ingest(record.table, record.op, record.id, record.record);
      }
    } finally {
      this.flushCoalescing();
    }
  }

  /**
   * Open a coalescing window. While open, the per-record stream updates
   * emitted by `ingest` are buffered (one entry per queryHash) instead of
   * dispatched. Always paired with `flushCoalescing()` in a try/finally by
   * `ingestMany` so the window always closes — otherwise the processor stays
   * stuck buffering forever.
   *
   * No-op if a window is already open (nested batches aren't expected here).
   */
  private beginCoalescing() {
    if (this.batching) return;
    this.batching = true;
    this.batchBuffer.clear();
  }

  /**
   * Close the coalescing window and flush: dispatch one coalesced
   * `StreamUpdate` per buffered queryHash, then persist processor state once
   * for the whole batch (instead of once per ingest).
   */
  private flushCoalescing() {
    if (!this.batching) return;
    this.batching = false;
    const buffered = Array.from(this.batchBuffer.values());
    this.batchBuffer.clear();
    if (buffered.length > 0) {
      this.dispatchUpdates(buffered);
    }
    // The processor state after the last ingest is cumulative, so a single
    // snapshot covers the whole batch. Kept fire-and-forget like the per-ingest
    // call it replaces.
    this.saveState();
  }

  /**
   * Initialize the WASM module and processor.
   * This must be called before using other methods.
   */
  async init() {
    if (this.isInitialized) return;

    this.logger.info(
      { Category: 'sp00ky-client::StreamProcessorService::init' },
      'Initializing WASM...'
    );
    try {
      await init(); // Initialize the WASM module (web target)
      // We cast the generated Sp00kyProcessor to our interface which is safer
      this.processor = new Sp00kyProcessor() as unknown as WasmProcessor;

      // Try to load state
      await this.loadState();

      this.isInitialized = true;
      this.logger.info(
        { Category: 'sp00ky-client::StreamProcessorService::init' },
        'Initialized successfully'
      );
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::StreamProcessorService::init' },
        'Failed to initialize'
      );
      throw e;
    }
  }

  async loadState() {
    if (!this.processor) return;
    try {
      const result = await this.persistenceClient.get('_00_stream_processor_state');

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
            Category: 'sp00ky-client::StreamProcessorService::loadState',
          },
          'Loading state from DB'
        );
        // Assuming processor has a load_state method matching the save_state behavior
        // If not, we might need to adjust based on the actual WASM API
        if (typeof this.processor.load_state === 'function') {
          this.processor.load_state(state);
        } else {
          this.logger.warn(
            { Category: 'sp00ky-client::StreamProcessorService::loadState' },
            'load_state method not found on processor'
          );
        }
      } else {
        this.logger.info(
          { Category: 'sp00ky-client::StreamProcessorService::loadState' },
          'No saved state found'
        );
      }
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::StreamProcessorService::loadState' },
        'Failed to load state'
      );
    }
  }

  /**
   * Seed per-table `select` permission predicates ({ [table]: whereText }).
   * Must run after the processor exists and before any `register_view`, else
   * non-`_00_` tables are default-denied and registration fails.
   */
  setPermissions(permissions: Record<string, string>) {
    if (!this.processor) return;
    if (typeof this.processor.set_permissions !== 'function') {
      this.logger.warn(
        { Category: 'sp00ky-client::StreamProcessorService::setPermissions' },
        'set_permissions not found on processor (stale WASM build?)'
      );
      return;
    }
    this.processor.set_permissions(permissions);
    this.logger.info(
      {
        tables: Object.keys(permissions).length,
        Category: 'sp00ky-client::StreamProcessorService::setPermissions',
      },
      'Seeded table permissions'
    );
  }

  async saveState() {
    if (!this.processor) return;
    try {
      // Assuming processor has a save_state method that returns the state string/bytes
      if (typeof this.processor.save_state === 'function') {
        const state = this.processor.save_state();
        if (state) {
          await this.persistenceClient.set('_00_stream_processor_state', state);
          this.logger.trace(
            { Category: 'sp00ky-client::StreamProcessorService::saveState' },
            'State saved'
          );
        }
      }
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::StreamProcessorService::saveState' },
        'Failed to save state'
      );
    }
  }

  /**
   * Ingest a record change into the processor.
   * Emits 'stream_update' event if materialized views are affected.
   * @param isOptimistic true = local mutation (increment versions), false = remote sync (keep versions)
   */
  ingest(
    table: string,
    op: 'CREATE' | 'UPDATE' | 'DELETE',
    id: string,
    record: any
  ): WasmStreamUpdate[] {
    this.logger.debug(
      {
        table,
        op,
        id,
        Category: 'sp00ky-client::StreamProcessorService::ingest',
      },
      'Ingesting into ssp'
    );

    if (!this.processor) {
      this.logger.warn(
        { Category: 'sp00ky-client::StreamProcessorService::ingest' },
        'Not initialized, skipping ingest'
      );
      return [];
    }

    try {
      const normalizedRecord = this.normalizeValue(record);

      const t0 = performance.now();
      const rawUpdates = this.processor.ingest(table, op, id, normalizedRecord);
      const materializationTimeMs = performance.now() - t0;
      this.logger.debug(
        {
          table,
          op,
          id,
          rawUpdates: rawUpdates.length,
          materializationTimeMs,
          Category: 'sp00ky-client::StreamProcessorService::ingest',
        },
        'Ingesting into ssp done'
      );

      if (rawUpdates && Array.isArray(rawUpdates) && rawUpdates.length > 0) {
        const updates: StreamUpdate[] = rawUpdates.map((u: WasmStreamUpdate) => ({
          queryHash: u.query_id,
          localArray: u.result_data,
          op: op,
          materializationTimeMs,
          storeApplyMs: u.timing_store_apply_ms,
          circuitStepMs: u.timing_circuit_step_ms,
          transformMs: u.timing_transform_ms,
        }));
        // Direct handler call instead of event
        this.notifyUpdates(updates);
      }
      // While batching (inside `ingestMany`), `flushCoalescing` persists once
      // for the whole batch — skip the redundant per-record snapshot here.
      if (!this.batching) {
        this.saveState();
      }
      return rawUpdates;
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::StreamProcessorService::ingest' },
        'Ingesting into ssp failed'
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
        { Category: 'sp00ky-client::StreamProcessorService::registerQueryPlan' },
        'Not initialized, skipping registration'
      );
      return;
    }

    this.logger.debug(
      {
        queryHash: queryPlan.queryHash,
        surql: queryPlan.surql,
        params: queryPlan.params,
        Category: 'sp00ky-client::StreamProcessorService::registerQueryPlan',
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
        { initialUpdate, Category: 'sp00ky-client::StreamProcessorService::registerQueryPlan' },
        'register_view result'
      );

      if (!initialUpdate) {
        throw new Error('Failed to register query plan');
      }
      const update: StreamUpdate = {
        queryHash: initialUpdate.query_id,
        localArray: initialUpdate.result_data,
        registration: {
          parseMs: initialUpdate.timing_parse_ms ?? 0,
          planMs: initialUpdate.timing_plan_ms ?? 0,
          snapshotMs: initialUpdate.timing_snapshot_ms ?? 0,
        },
      };
      this.saveState();
      this.logger.debug(
        {
          queryHash: queryPlan.queryHash,
          surql: queryPlan.surql,
          params: queryPlan.params,
          Category: 'sp00ky-client::StreamProcessorService::registerQueryPlan',
        },
        'Registered query plan'
      );
      return update;
    } catch (e) {
      this.logger.error(
        { error: e, Category: 'sp00ky-client::StreamProcessorService::registerQueryPlan' },
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
        { error: e, Category: 'sp00ky-client::StreamProcessorService::unregisterQueryPlan' },
        'Error unregistering query plan'
      );
    }
  }

  private normalizeValue(value: any): any {
    if (value === null || value === undefined) return value;

    if (typeof value === 'object') {
      // CRDT snapshots arrive as `Uint8Array` (or `ArrayBuffer` /
      // typed-array views). `serde_wasm_bindgen::from_value` rejects
      // those when deserializing into `serde_json::Value` (JSON has no
      // binary variant), and the SSP can't filter on opaque bytes
      // anyway. Replace with `null` so the row still flows through the
      // ingest path with its other columns intact, and downstream
      // predicates referencing the bytes column simply don't match.
      if (
        value instanceof Uint8Array ||
        value instanceof ArrayBuffer ||
        ArrayBuffer.isView(value)
      ) {
        return null;
      }

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
          { result, Category: 'sp00ky-client::StreamProcessorService::normalizeValue' },
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
