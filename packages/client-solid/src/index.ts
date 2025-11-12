import type { SyncedDbConfig } from "./types";
import {
  RecordId,
  GetTable,
  QueryBuilder,
  SchemaStructure,
  TableModel,
  TableNames,
  createSpooky,
  Surreal,
} from "@spooky/spooky-ts";
import { Effect } from "effect";
export type { RecordResult } from "surrealdb";

export { RecordId } from "surrealdb";
export type {
  Model,
  GenericModel,
  GenericSchema,
  ModelPayload,
} from "./lib/models";
export { useQuery } from "./lib/use-query";

// Re-export query builder types for convenience
export type {
  QueryModifier,
  QueryModifierBuilder,
  QueryInfo,
  RelationshipsMetadata,
  RelationshipDefinition,
  InferRelatedModelFromMetadata,
  GetCardinality,
  GetTable,
  TableModel,
  TableNames,
} from "@spooky/query-builder";

export interface SyncedDbContext<S extends SyncedDb<SchemaStructure>> {
  db: S;
}

/**
 * SyncedDb - A thin wrapper around spooky-ts for Solid.js integration
 * Delegates all logic to the underlying spooky-ts instance
 */
export class SyncedDb<const Schema extends SchemaStructure> {
  private config: SyncedDbConfig<Schema>;
  private spooky: Awaited<ReturnType<typeof createSpooky<Schema>>> | null =
    null;

  constructor(config: SyncedDbConfig<Schema>) {
    this.config = config;
  }

  /**
   * Initialize the spooky-ts instance
   */
  async init(): Promise<void> {
    this.spooky = await createSpooky<Schema>(this.config);
  }

  /**
   * Create a new record in the database
   */
  async create<TName extends TableNames<Schema>>(
    tableName: TName,
    payload: TableModel<GetTable<Schema, TName>>
  ): Promise<void> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    await Effect.runPromise(this.spooky.create(tableName, payload));
  }

  /**
   * Query data from the database
   */
  public query<TName extends TableNames<Schema>>(
    table: TName
  ): QueryBuilder<Schema, TName, {}, false> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    return Effect.runSync(this.spooky.query(table, {}));
  }

  /**
   * Query data from the database
   */
  public async authenticate(token: string): Promise<RecordId<string>> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    const userId = await Effect.runPromise(this.spooky.authenticate(token));
    return userId;
  }

  public async deauthenticate(): Promise<void> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    await Effect.runPromise(this.spooky.deauthenticate());
  }

  public db(): Surreal {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    return Effect.runSync(this.spooky.useRemote((db) => db));
  }
}

export * from "./types";
