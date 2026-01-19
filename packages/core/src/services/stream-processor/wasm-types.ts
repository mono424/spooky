import { RecordVersionArray } from '../../types.js';

export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_data: RecordVersionArray; // Match Rust 'result_data' field
}

export interface WasmIncantationConfig {
  id: string;
  surrealQL: string;
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

// Interface matching the SpookyProcessor class from WASM
export interface WasmProcessor {
  ingest(
    table: string,
    op: string,
    id: string,
    record: any,
    isOptimistic: boolean
  ): WasmStreamUpdate[];
  ingest_batch(batch: WasmIngestItem[], isOptimistic: boolean): WasmStreamUpdate[];
  set_record_version(
    incantation_id: string,
    record_id: string,
    version: number
  ): WasmStreamUpdate | undefined;
  register_view(config: WasmIncantationConfig): WasmStreamUpdate | undefined;
  unregister_view(id: string): void;
  // Add other methods if needed
}
