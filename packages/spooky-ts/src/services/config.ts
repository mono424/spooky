import { SchemaStructure } from "@spooky/query-builder";
import { Context, Effect, Layer } from "effect";
import { ProvisionOptions } from "src/provision.js";

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

export class Config extends Context.Tag("Config")<
  Config,
  {
    readonly getConfig: Effect.Effect<SpookyConfig<SchemaStructure>>;
  }
>() {}

export const makeConfig = <S extends SchemaStructure>() =>
  Context.GenericTag<{
    readonly getConfig: Effect.Effect<SpookyConfig<S>>;
  }>("Config");

export const ConfigLayer = <S extends SchemaStructure>(
  config: SpookyConfig<S>
) =>
  Layer.succeed(makeConfig<S>(), {
    getConfig: Effect.succeed(config),
  });
