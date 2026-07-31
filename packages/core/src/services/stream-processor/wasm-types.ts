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

// Interface matching the Sp00kyProcessor class from WASM
export interface WasmProcessor {
  ingest(table: string, op: string, id: string, record: any): WasmStreamUpdate[];
  register_view(config: WasmQueryConfig): WasmStreamUpdate | undefined;
  unregister_view(id: string): void;
  // Seed per-table `select` permission predicates ({ [table]: whereText }) so
  // register_view can inject them instead of default-denying the table.
  set_permissions(permissions: Record<string, string>): void;
  // Persistence hooks: present on current WASM builds, absent on stale ones.
  // Always guarded with `typeof x === 'function'` before calling so an older
  // build degrades gracefully instead of throwing.
  load_state?(state: string): void;
  save_state?(): string;
  // wasm-bindgen destructor. Releases the circuit (store + every view cache)
  // inside wasm linear memory. Without it the bytes only come back when V8
  // happens to GC the JS wrapper, which it has no reason to hurry since it
  // cannot see how much wasm memory the wrapper is holding.
  free?(): void;
}
