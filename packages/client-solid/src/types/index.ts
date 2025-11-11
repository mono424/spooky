import type { Surreal } from "surrealdb";
import type { SyncedDb } from "../index";
import { GenericSchema } from "../lib/models";
import type {
  SchemaStructure,
  TableNames,
  GetTable,
  TableModel,
} from "@spooky/query-builder";

/**
 * Options for database provisioning
 */
export interface ProvisionOptions {
  /** Force re-provision even if schema already exists */
  force?: boolean;
}

declare global {
  interface Window {
    db?: SyncedDb<any>;
  }
}

export type CacheStrategy = "memory" | "indexeddb";

/**
 * Infer Schema type (Record<TableName, Model>) from schema const
 */
export type InferSchemaFromConst<S extends SchemaStructure> = {
  [K in TableNames<S>]: TableModel<GetTable<S, K>>;
};

/**
 * Infer Relationships type from schema const's relationships array
 * Converts from array format to nested object format
 */
export type InferRelationshipsFromConst<
  S extends SchemaStructure,
  Schema extends GenericSchema
> = {
  [TableName in TableNames<S>]: {
    [Rel in Extract<
      S["relationships"][number],
      { from: TableName }
    > as Rel["field"]]: {
      model: Rel["to"] extends keyof Schema ? Schema[Rel["to"]] : any;
      table: Rel["to"];
      cardinality: Rel["cardinality"];
    };
  };
};

export interface SyncedDbConfig<S extends SchemaStructure> {
  /** Schema const with runtime metadata (tables and relationships) */
  schema: S;
  /** SurrealQL schema string for database provisioning */
  schemaSurql: string;
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
  /** Provision options */
  provisionOptions?: ProvisionOptions;
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
