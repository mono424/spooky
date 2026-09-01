import type { RecordVersionArray } from '../../types';

export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_data: RecordVersionArray; // Match Rust 'result_data' field
  // Per-phase SSP processing time (ms). Ingest path: store_apply/circuit_step/
  // transform. Register path: parse/plan/snapshot. Unused side is 0.
  timing_store_apply_ms?: number;
  timing_circuit_step_ms?: number;
  timing_transform_ms?: number;
  timing_parse_ms?: number;
  timing_plan_ms?: number;
  timing_snapshot_ms?: number;
}

export interface WasmQueryConfig {
  id: string;
  surql: string;
  params?: Record<string, any>;
  clientId: string;
  ttl: string;
  lastActiveAt: string;
}

export interface WasmIngestItem {
  table: string;
  op: string;
  id: string;
  record: any;
  version?: number;
}

/** `register_view`'s result: the initial update plus, under projection, the
 *  fields this plan evaluates that already-stored rows were kept without. */
export interface WasmRegistration extends WasmStreamUpdate {
  missing_fields?: Record<string, string[]>;
}

/** `reconcile`'s result. */
export interface WasmReconciled {
  /** Ids (caller's spelling) whose body the store lacks or holds stale. */
  fetch: string[];
  /** Rows deleted because the caller's list did not have them. */
  deleted: number;
  /** View updates produced by those deletes. */
  updates: WasmStreamUpdate[];
}

// Interface matching the Sp00kyProcessor class from WASM
export interface WasmProcessor {
  ingest(table: string, op: string, id: string, record: any): WasmStreamUpdate[];
  // Bulk ingest: ONE circuit step for the whole array, returning the coalesced
  // updates. Optional because a stale WASM build won't have it — callers guard
  // with `typeof x === 'function'` and fall back to a loop over `ingest`.
  ingest_many?(items: WasmIngestItem[]): WasmStreamUpdate[];
  register_view(config: WasmQueryConfig): WasmRegistration | undefined;
  unregister_view(id: string): void;
  // Seed per-table `select` permission predicates ({ [table]: whereText }) so
  // register_view can inject them instead of default-denying the table.
  set_permissions(permissions: Record<string, string>): void;
  // Persistence hooks: present on current WASM builds, absent on stale ones.
  // Always guarded with `typeof x === 'function'` before calling so an older
  // build degrades gracefully instead of throwing.
  load_state?(state: string): void;
  save_state?(): string;
  // Store-only snapshot as bytes (views are re-registered per session, so
  // they are never persisted). `load_store_state` installs it UNDER the views
  // already registered and returns their re-primed results.
  save_store_state?(): Uint8Array;
  load_store_state?(bytes: Uint8Array): WasmStreamUpdate[];
  // Client-side `_00_rv` catch-up against the durable store's `[id, rv][]`.
  reconcile?(table: string, entries: [string, number][]): WasmReconciled;
  max_row_versions?(): Record<string, number>;
  // Row-arena hygiene. `compact` decodes every row, so checkpoint-time only.
  compact?(): number;
  dead_bytes?(): number;
  live_bytes?(): number;
  // Keep only the fields registered plans evaluate per stored row.
  set_projection?(enabled: boolean): void;
  size_report?(): unknown;
  // wasm-bindgen destructor. Releases the circuit (store + every view cache)
  // inside wasm linear memory. Without it the bytes only come back when V8
  // happens to GC the JS wrapper, which it has no reason to hurry since it
  // cannot see how much wasm memory the wrapper is holding.
  free?(): void;
}
