export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_ids: string[];
  tree: any; // Wasm tree structure
}

export interface WasmIncantationConfig {
  id: string;
  surrealQL: string;
  params?: Record<string, any>;
  clientId: string;
  ttl: string;
  lastActiveAt: string;
}

// Interface matching the SpookyProcessor class from WASM
export interface WasmProcessor {
  ingest(table: string, op: string, id: string, record: any): WasmStreamUpdate[];
  register_view(config: WasmIncantationConfig): WasmStreamUpdate | undefined;
  unregister_view(id: string): void;
  // Add other methods if needed
}
