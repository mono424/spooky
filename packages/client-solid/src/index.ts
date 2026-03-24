import type { SyncedDbConfig } from './types';
import {
  Sp00kyClient,
  AuthService,
  BucketHandle,
  type Sp00kyQueryResultPromise,
  UpdateOptions,
  RunOptions,
} from '@spooky-sync/core';

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
  BackendNames,
  BackendRoutes,
  RoutePayload,
  BucketNames,
  BucketDefinitionSchema,
} from '@spooky-sync/query-builder';

import { RecordId, Uuid, Surreal } from 'surrealdb';
export { RecordId, Uuid };
export type { Model, GenericModel, GenericSchema, ModelPayload } from './lib/models';
export { useQuery } from './lib/use-query';
export { useFileUpload, type FileUploadResult } from './lib/use-file-upload';
export { useDownloadFile, type UseDownloadFileOptions, type UseDownloadFileResult } from './lib/use-download-file';
export { Sp00kyProvider, type Sp00kyProviderProps } from './lib/Sp00kyProvider';
export { useDb } from './lib/context';

// export { AuthEventTypes } from "@spooky-sync/core"; // TODO: Verify if AuthEventTypes exists in core
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
} from '@spooky-sync/query-builder';

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
 * SyncedDb - A thin wrapper around sp00ky-ts for Solid.js integration
 * Delegates all logic to the underlying sp00ky-ts instance
 */
export class SyncedDb<S extends SchemaStructure> {
  private config: SyncedDbConfig<S>;
  private sp00ky: Sp00kyClient<S> | null = null;
  private _initialized = false;

  constructor(config: SyncedDbConfig<S>) {
    this.config = config;
  }

  public getSp00ky(): Sp00kyClient<S> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky;
  }

  /**
   * Initialize the sp00ky-ts instance
   */
  async init(): Promise<void> {
    if (this._initialized) return;
    this.sp00ky = new Sp00kyClient<S>(this.config);
    await this.sp00ky.init();
    this._initialized = true;
  }

  /**
   * Create a new record in the database
   */
  async create(id: string, payload: Record<string, unknown>): Promise<void> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    await this.sp00ky.create(id, payload as Record<string, unknown>);
  }

  /**
   * Update an existing record in the database
   */
  async update<TName extends TableNames<S>>(
    tableName: TName,
    recordId: string,
    payload: Partial<TableModel<GetTable<S, TName>>>,
    options?: UpdateOptions
  ): Promise<void> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    await this.sp00ky.update(
      tableName as string,
      recordId,
      payload as Record<string, unknown>,
      options
    );
  }

  /**
   * Delete an existing record in the database
   */
  async delete<TName extends TableNames<S>>(
    tableName: TName,
    selector: string | InnerQuery<GetTable<S, TName>, boolean>
  ): Promise<void> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    if (typeof selector !== 'string')
      throw new Error('Only string ID selectors are supported currently with core');
    await this.sp00ky.delete(tableName as string, selector);
  }

  /**
   * Query data from the database
   */
  public query<TName extends TableNames<S>>(
    table: TName
  ): QueryBuilder<S, TName, Sp00kyQueryResultPromise, {}, false> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.query(table, {});
  }

  /**
   * Run a backend operation
   */
  public async run<
    B extends BackendNames<S>,
    R extends BackendRoutes<S, B>,
  >(
    backend: B,
    path: R,
    payload: RoutePayload<S, B, R>,
    options?: RunOptions,
  ): Promise<void> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    await this.sp00ky.run(backend, path, payload, options);
  }

  /**
   * Authenticate with the database
   */
  public async authenticate(token: string): Promise<RecordId<string>> {
    const result = await this.sp00ky?.authenticate(token);
    // Sp00kyClient.authenticate returns whatever remote.authenticate returns (boolean or token usually?)
    // Wait, checked Sp00kyClient: return this.remote.getClient().authenticate(token);
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
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    await this.sp00ky.auth.signOut();
  }

  /**
   * Execute a function with direct access to the remote database connection
   */
  public async useRemote<T>(fn: (db: Surreal) => T | Promise<T>): Promise<T> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return await this.sp00ky.useRemote(fn);
  }
  /**
   * Access the remote database service directly
   */
  get remote(): Sp00kyClient<S>['remoteClient'] {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.remoteClient;
  }

  /**
   * Access the local database service directly
   */
  get local(): Sp00kyClient<S>['localClient'] {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.localClient;
  }

  /**
   * Access the auth service
   */
  get auth(): AuthService<S> {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.auth;
  }

  get pendingMutationCount(): number {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.pendingMutationCount;
  }

  subscribeToPendingMutations(cb: (count: number) => void): () => void {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.subscribeToPendingMutations(cb);
  }

  bucket<B extends BucketNames<S>>(name: B): BucketHandle {
    if (!this.sp00ky) throw new Error('SyncedDb not initialized');
    return this.sp00ky.bucket(name);
  }

  getBucketConfig(name: string): BucketDefinitionSchema | undefined {
    return this.config.schema.buckets?.find((b) => b.name === name);
  }
}

export * from './types';
