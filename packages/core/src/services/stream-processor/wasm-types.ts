import { RecordVersionArray } from '../../types.js';

export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_data: RecordVersionArray; // Match Rust 'result_data' field
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

// Interface matching the SpookyProcessor class from WASM
export interface WasmProcessor {
  ingest(table: string, op: string, id: string, record: any): WasmStreamUpdate[];
  register_view(config: WasmQueryConfig): WasmStreamUpdate | undefined;
  unregister_view(id: string): void;
}
