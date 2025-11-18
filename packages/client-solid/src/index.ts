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

export { RecordId, Uuid } from "surrealdb";
export type {
  Model,
  GenericModel,
  GenericSchema,
  ModelPayload,
} from "./lib/models";
export { useQuery } from "./lib/use-query";

export { AuthEventTypes } from "@spooky/spooky-ts";

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

  public getSpooky(): Awaited<ReturnType<typeof createSpooky<Schema>>> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    return this.spooky;
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
    await this.spooky.create(tableName, payload);
  }

  /**
   * Query data from the database
   */
  public query<TName extends TableNames<Schema>>(
    table: TName
  ): QueryBuilder<Schema, TName, {}, false> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    return this.spooky.query(table, {});
  }

  /**
   * Authenticate with the database
   */
  public async authenticate(token: string): Promise<RecordId<string>> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    const userId = await this.spooky.authenticate(token);
    return userId!;
  }

  /**
   * Deauthenticate from the database
   */
  public async deauthenticate(): Promise<void> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    await this.spooky.deauthenticate();
  }

  /**
   * Execute a function with direct access to the remote database connection
   */
  public async useRemote<T>(fn: (db: Surreal) => T | Promise<T>): Promise<T> {
    if (!this.spooky) throw new Error("SyncedDb not initialized");
    return await this.spooky.useRemote(fn);
  }
}

export * from "./types";
