/* tslint:disable */
/* eslint-disable */

export interface WasmViewUpdate {
    query_id: string;
    result_hash: string;
    result_data: [string, number][];
    delta: {
        additions: [string, number][];
        removals: string[];
        updates: [string, number][];
    };
    // Per-phase SSP processing time (ms). Ingest path: store_apply/circuit_step/
    // transform. Register path: parse/plan/snapshot. Unused side is 0.
    timing_store_apply_ms: number;
    timing_circuit_step_ms: number;
    timing_transform_ms: number;
    timing_parse_ms: number;
    timing_plan_ms: number;
    timing_snapshot_ms: number;
}

export interface WasmViewConfig {
    id: string;
    surql: string;
    params?: Record<string, any>;
    clientId: string;
    ttl: string;
    lastActiveAt: string;
    safe_params?: Record<string, any>;
    format?: 'flat' | 'tree' | 'streaming';
}

export interface WasmIngestItem {
    table: string;
    op: string;
    id: string;
    record: any;
}

export interface WasmRegistration extends WasmViewUpdate {
    /** Under projection: fields this plan evaluates that stored rows lack. */
    missing_fields?: Record<string, string[]>;
}

export interface WasmReconciled {
    fetch: string[];
    deleted: number;
    updates: WasmViewUpdate[];
}



export class Sp00kyProcessor {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Rebuild row storage without the bytes orphaned by updates and deletes.
     * Returns how many bytes were dead. Costs a decode of every row, so call
     * it from a checkpoint, never per ingest.
     */
    compact(): number;
    /**
     * Bytes of row storage orphaned by updates and deletes.
     */
    dead_bytes(): number;
    /**
     * Ingest a record into the stream processor
     */
    ingest(table: string, op: string, id: string, record: any): any;
    /**
     * Ingest MANY record changes as ONE circuit step.
     *
     * `ingest` costs one full circuit step per record, and a step walks every
     * registered view, so a cold sync that lands thousands of rows paid that
     * fixed cost thousands of times (a ~3.9k-row registry took ~3.4s of circuit
     * time on a laptop, ~0.85ms a row, nearly all of it per-step overhead).
     * `ChangeSet` already carries many changes and `step_timed` applies them
     * all to the store before stepping once, so a batch is a single step with
     * one set of deltas.
     *
     * Same input shape as `ingest`, as an array: `WasmIngestItem[]`. Returns
     * the coalesced `WasmViewUpdate[]` for the whole batch. Changes are applied
     * in array order, so repeated ids inside one batch settle last-write-wins,
     * exactly as sequential `ingest` calls would.
     */
    ingest_many(items: any): any;
    /**
     * Bytes of row storage referenced by live rows.
     */
    live_bytes(): number;
    /**
     * Load circuit state from a JSON string
     */
    load_state(state: string): void;
    /**
     * Install a snapshot written by `save_store_state` UNDER the views that
     * are already registered, keeping permissions and projection. Every
     * registered view is re-primed against the restored rows; the returned
     * `WasmViewUpdate[]` carries their new full results, so a query that
     * registered against the empty pre-snapshot store catches up.
     */
    load_store_state(bytes: Uint8Array): any;
    /**
     * Highest `_00_rv` folded into each table, `{ [table]: rv }`.
     */
    max_row_versions(): any;
    constructor();
    /**
     * Compare one table against the caller's authoritative `[id, rv][]`.
     * Rows the store holds but the list lacks are deleted (with view
     * updates); ids the store lacks or holds at a lower `_00_rv` come back in
     * `fetch` for the caller to ingest. See `Circuit::reconcile`.
     */
    reconcile(table: string, entries: any): any;
    /**
     * Register a new materialized view
     */
    register_view(config: any): any;
    /**
     * Save the current circuit state as a JSON string
     */
    save_state(): string;
    /**
     * Snapshot the base collections only, as bytes (a `Uint8Array` in JS).
     * Views are deliberately left out: the client re-registers every query
     * under a fresh session id on boot, so persisted views would only be
     * stepped and never read. Pair with `load_store_state`.
     */
    save_store_state(): Uint8Array;
    /**
     * Seed per-table `select` permission predicates so `register_view` can
     * inject them (and so non-`_00_` tables aren't default-denied).
     *
     * Expects a `{ [table]: whereText }` object, where `whereText` is the raw
     * `WHERE` expression from the table's `PERMISSIONS FOR select` clause
     * (e.g. `"true"`, or `"owner = $auth.id"`). Called once at boot after the
     * schema is parsed — mirrors the native boot path that reads `INFO FOR DB`.
     */
    set_permissions(permissions: any): void;
    /**
     * Keep only the fields registered plans evaluate (plus `id`/`_00_rv`)
     * per stored row. Off by default. Must be set before the first ingest
     * to take effect on those rows; `compact` re-projects existing ones.
     */
    set_projection(enabled: boolean): void;
    /**
     * Per-table and per-view heap attribution, sorted heaviest first.
     */
    size_report(): any;
    /**
     * Unregister a view by ID
     */
    unregister_view(id: string): void;
}

/**
 * Called when WASM module is loaded
 */
export function init(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_sp00kyprocessor_free: (a: number, b: number) => void;
    readonly init: () => void;
    readonly sp00kyprocessor_compact: (a: number) => number;
    readonly sp00kyprocessor_dead_bytes: (a: number) => number;
    readonly sp00kyprocessor_ingest: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: any) => [number, number, number];
    readonly sp00kyprocessor_ingest_many: (a: number, b: any) => [number, number, number];
    readonly sp00kyprocessor_live_bytes: (a: number) => number;
    readonly sp00kyprocessor_load_state: (a: number, b: number, c: number) => [number, number];
    readonly sp00kyprocessor_load_store_state: (a: number, b: number, c: number) => [number, number, number];
    readonly sp00kyprocessor_max_row_versions: (a: number) => [number, number, number];
    readonly sp00kyprocessor_new: () => number;
    readonly sp00kyprocessor_reconcile: (a: number, b: number, c: number, d: any) => [number, number, number];
    readonly sp00kyprocessor_register_view: (a: number, b: any) => [number, number, number];
    readonly sp00kyprocessor_save_state: (a: number) => [number, number, number, number];
    readonly sp00kyprocessor_save_store_state: (a: number) => [number, number, number, number];
    readonly sp00kyprocessor_set_permissions: (a: number, b: any) => [number, number];
    readonly sp00kyprocessor_set_projection: (a: number, b: number) => void;
    readonly sp00kyprocessor_size_report: (a: number) => [number, number, number];
    readonly sp00kyprocessor_unregister_view: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
