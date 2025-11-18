import { SchemaStructure } from "@spooky/query-builder";
import { ProvisionOptions } from "../provision.js";

export type CacheStrategy = "memory" | "indexeddb";

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface SpookyConfig<S extends SchemaStructure> {
  /** Schema const with runtime metadata (tables and relationships) */
  schema: S;
  /** SurrealQL schema string for database provisioning */
  schemaSurql: string;
  /** Remote database URL - required for sync functionality */
  remoteUrl: string;
  /** Local database name for WASM storage */
  localDbName: string;
  /** Internal database name for WASM storage */
  internalDbName: string;
  /** Storage strategy for SurrealDB WASM */
  storageStrategy: CacheStrategy;
  /** Namespace for the database */
  namespace: string;
  /** Database name */
  database: string;
  /** Provision options */
  provisionOptions: ProvisionOptions;
  /** Log level */
  logLevel: LogLevel;
}
