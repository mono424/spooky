import type { Surreal } from "surrealdb";
import type { SyncedDb } from "../index";
import { GenericSchema } from "../lib/models";

declare global {
  interface Window {
    db?: SyncedDb<any>;
  }
}

export type CacheStrategy = "memory" | "indexeddb";

export interface SyncedDbConfig<Schema extends GenericSchema> {
  tables: (keyof Schema & string)[];
  /** Schema for the database */
  schema: string;
  /** Remote database URL (optional) */
  remoteUrl?: string;
  /** Local database name for WASM storage */
  localDbName: string;
  /** Internal database name for WASM storage */
  internalDbName: string;
  /** Storage strategy for SurrealDB WASM */
  storageStrategy: CacheStrategy;
  /** Namespace for the database */
  namespace?: string;
  /** Database name */
  database?: string;
  /** Relationships metadata for automatic RecordId conversion */
  relationships?: Record<string, Array<{ field: string; table: string; cardinality?: "one" | "many" }>>;
}

export interface LocalDbConfig {
  name: string;
  storageStrategy: CacheStrategy;
  namespace?: string;
  database?: string;
}

export interface RemoteDbConfig {
  url: string;
  token?: string;
  namespace?: string;
  database?: string;
}

export interface DbConnection {
  internal: Surreal;
  local: Surreal;
  remote?: Surreal;
}

export interface SyncStatus {
  isConnected: boolean;
  lastSync?: Date;
  pendingChanges: number;
  localRecords: number;
  remoteRecords?: number;
}

export interface SyncOptions {
  /** Force full sync regardless of last sync time */
  force?: boolean;
  /** Sync only specific tables */
  tables?: string[];
  /** Batch size for sync operations */
  batchSize?: number;
}
