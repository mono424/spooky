import type { Surreal } from "surrealdb";
import type { SyncedDb } from "../index";

declare global {
  interface Window {
    db?: SyncedDb<any>;
  }
}

export type CacheStrategy = "memory" | "indexeddb";

export interface SyncedDbConfig {
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
  /** Authentication token for remote database */
  token?: string;
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

export type * from "./models";
