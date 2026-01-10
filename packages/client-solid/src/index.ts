import type { SyncedDbConfig } from './types';
import { SpookyClient, AuthService, type SpookyQueryResultPromise } from '@spooky/core';

import {
  GetTable,
  QueryBuilder,
  SchemaStructure,
  TableModel,
  TableNames,
  QueryResult,
  RelatedFieldsMap,
  RelationshipFieldsFromSchema,
  GetRelationship,
  RelatedFieldMapEntry,
  InnerQuery,
} from '@spooky/query-builder';

import { RecordId, Uuid, Surreal } from 'surrealdb';
export { RecordId, Uuid };
export type { Model, GenericModel, GenericSchema, ModelPayload } from './lib/models';
export { useQuery } from './lib/use-query';

// export { AuthEventTypes } from "@spooky/core"; // TODO: Verify if AuthEventTypes exists in core
export type {};

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
  QueryResult,
} from '@spooky/query-builder';

export interface SyncedDbContext<S extends SyncedDb<SchemaStructure>> {
  db: S;
}

export type RelationshipField<
  Schema extends SchemaStructure,
  TableName extends TableNames<Schema>,
  Field extends RelationshipFieldsFromSchema<Schema, TableName>,
> = GetRelationship<Schema, TableName, Field>;

export type RelatedFieldsTableScoped<
  Schema extends SchemaStructure,
  TableName extends TableNames<Schema>,
  RelatedFields extends RelationshipFieldsFromSchema<Schema, TableName> =
    RelationshipFieldsFromSchema<Schema, TableName>,
> = {
  [K in RelatedFields]: {
    to: RelationshipField<Schema, TableName, K>['to'];
    relatedFields: RelatedFieldsMap;
    cardinality: RelationshipField<Schema, TableName, K>['cardinality'];
  };
};

export type InferModel<
  Schema extends SchemaStructure,
  TableName extends TableNames<Schema>,
  RelatedFields extends RelatedFieldsTableScoped<Schema, TableName>,
> = QueryResult<Schema, TableName, RelatedFields, true>;

export type WithRelated<Field extends string, RelatedFields extends RelatedFieldsMap = {}> = {
  [K in Field]: Omit<RelatedFieldMapEntry, 'relatedFields'> & {
    relatedFields: RelatedFields;
  };
};

export type WithRelatedMany<Field extends string, RelatedFields extends RelatedFieldsMap = {}> = {
  [K in Field]: {
    to: Field;
    relatedFields: RelatedFields;
    cardinality: 'many';
  };
};

/**
 * SyncedDb - A thin wrapper around spooky-ts for Solid.js integration
 * Delegates all logic to the underlying spooky-ts instance
 */
export class SyncedDb<S extends SchemaStructure> {
  private config: SyncedDbConfig<S>;
  private spooky: SpookyClient<S> | null = null;

  constructor(config: SyncedDbConfig<S>) {
    this.config = config;
  }

  public getSpooky(): SpookyClient<S> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return this.spooky;
  }

  /**
   * Initialize the spooky-ts instance
   */
  async init(): Promise<void> {
    this.spooky = new SpookyClient<S>(this.config);
    await this.spooky.init();
  }

  /**
   * Create a new record in the database
   */
  async create(id: string, payload: Record<string, unknown>): Promise<void> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    await this.spooky.create(id, payload as Record<string, unknown>);
  }

  /**
   * Update an existing record in the database
   */
  async update<TName extends TableNames<S>>(
    tableName: TName,
    recordId: string,
    payload: Partial<TableModel<GetTable<S, TName>>>
  ): Promise<void> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    await this.spooky.update(tableName as string, recordId, payload as Record<string, unknown>);
  }

  /**
   * Delete an existing record in the database
   */
  async delete<TName extends TableNames<S>>(
    tableName: TName,
    selector: string | InnerQuery<GetTable<S, TName>, boolean>
  ): Promise<void> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    if (typeof selector !== 'string')
      throw new Error('Only string ID selectors are supported currently with core');
    await this.spooky.delete(tableName as string, selector);
  }

  /**
   * Query data from the database
   */
  public query<TName extends TableNames<S>>(
    table: TName
  ): QueryBuilder<S, TName, SpookyQueryResultPromise, {}, false> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return this.spooky.query(table, {});
  }

  /**
   * Authenticate with the database
   */
  public async authenticate(token: string): Promise<RecordId<string>> {
    const result = await this.spooky?.authenticate(token);
    // SpookyClient.authenticate returns whatever remote.authenticate returns (boolean or token usually?)
    // Wait, checked SpookyClient: return this.remote.getClient().authenticate(token);
    // SurrealDB authenticate returns void? or token?
    // Assuming void or token.
    return new RecordId('user', 'me'); // Placeholder or actual?
  }

  /**
   * Deauthenticate from the database
   * @deprecated Use signOut() instead
   */
  public async deauthenticate(): Promise<void> {
    await this.signOut();
  }

  /**
   * Sign out, clear session and local storage
   */
  public async signOut(): Promise<void> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    await this.spooky.auth.signOut();
  }

  /**
   * Execute a function with direct access to the remote database connection
   */
  public async useRemote<T>(fn: (db: Surreal) => T | Promise<T>): Promise<T> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return await this.spooky.useRemote(fn);
  }
  /**
   * Access the remote database service directly
   */
  get remote(): SpookyClient<S>['remoteClient'] {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return this.spooky.remoteClient;
  }

  /**
   * Access the local database service directly
   */
  get local(): SpookyClient<S>['localClient'] {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return this.spooky.localClient;
  }

  /**
   * Access the auth service
   */
  get auth(): AuthService<S> {
    if (!this.spooky) throw new Error('SyncedDb not initialized');
    return this.spooky.auth;
  }
}

export * from './types';
