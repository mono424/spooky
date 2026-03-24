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



export class Sp00kyProcessor {
  free(): void;
  [Symbol.dispose](): void;
  /**
   * Load circuit state from a JSON string
   */
  load_state(state: string): void;
  /**
   * Save the current circuit state as a JSON string
   */
  save_state(): string;
  /**
   * Register a new materialized view
   */
  register_view(config: any): any;
  /**
   * Unregister a view by ID
   */
  unregister_view(id: string): void;
  constructor();
  /**
   * Ingest a record into the stream processor
   */
  ingest(table: string, op: string, id: string, record: any): any;
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
  readonly sp00kyprocessor_ingest: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: any) => [number, number, number];
  readonly sp00kyprocessor_load_state: (a: number, b: number, c: number) => [number, number];
  readonly sp00kyprocessor_new: () => number;
  readonly sp00kyprocessor_register_view: (a: number, b: any) => [number, number, number];
  readonly sp00kyprocessor_save_state: (a: number) => [number, number, number, number];
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
