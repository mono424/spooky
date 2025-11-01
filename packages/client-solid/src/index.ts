import {
  RecordId,
  Surreal,
  Table,
  Values,
  createRemoteEngines,
  type AnyAuth,
  type AccessRecordAuth,
  type AuthResponse,
} from "surrealdb";
import { SchemaProvisioner } from "./lib/provisioner";
import { createSurrealDBWasm } from "./cache";
import type { SyncedDbConfig, DbConnection } from "./types";
import { GenericModel, ModelPayload } from "./lib/models";
import { Syncer } from "./lib/syncer";
import { WALManager } from "./lib/wal";
import { proxy } from "valtio";
import {
  GetTable,
  QueryBuilder,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { Executer } from "./lib/executer";
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

export type QueryResponse<T> = Omit<ReturnType<Surreal["query"]>, "collect"> & {
  collect: () => Promise<[ModelPayload<T>[]]>;
};

/**
 * Recursively convert DateTime objects to Date objects and RecordId objects to strings
 */
function convertDateTimeToDate(value: any): any {
  // Convert RecordId to string
  if (value instanceof RecordId) {
    return value.toString();
  }
  // Process arrays recursively
  if (Array.isArray(value)) {
    return value.map(convertDateTimeToDate);
  }
  // Process plain objects recursively
  if (value && typeof value === "object" && value.constructor === Object) {
    const result: any = {};
    for (const key in value) {
      if (Object.prototype.hasOwnProperty.call(value, key)) {
        result[key] = convertDateTimeToDate(value[key]);
      }
    }
    return result;
  }
  return value;
}

/**
 * Process model: convert DateTime to Date and RecordId objects to strings
 */
function wrapModelIdWithRef<T extends GenericModel | undefined>(model: T): T {
  if (!model) return model;

  // Convert DateTime to Date and RecordId to string (including id and relationship fields)
  return convertDateTimeToDate(model) as T;
}

export interface SyncedDbContext<S extends SyncedDb<SchemaStructure>> {
  db: S;
}

export class SyncedDb<const Schema extends SchemaStructure> {
  private config: SyncedDbConfig<Schema>;
  private connections: DbConnection | null = null;
  private executer: Executer<Schema>;
  private syncer: Syncer | null = null;
  private walManager: WALManager | null = null;
  private currentUser: {
    value: TableModel<GetTable<Schema, TableNames<Schema>>> | null;
  } = proxy({ value: null });

  constructor(config: SyncedDbConfig<Schema>) {
    this.config = config;
    this.executer = new Executer<Schema>(this);
  }

  public query<TName extends TableNames<Schema>>(
    table: TName
  ): QueryBuilder<Schema, TName, false> {
    return new QueryBuilder(
      this.config.schema,
      table,
      this.executer.run.bind(this.executer)
    );
  }

  /**
   * Initialize local WASM DB and optional remote client, then provision local schema
   */
  async init(): Promise<void> {
    const {
      localDbName,
      internalDbName,
      storageStrategy,
      namespace,
      database,
      remoteUrl,
      schemaSurql,
    } = this.config;

    // Internal WASM database
    const internal = await createSurrealDBWasm(
      internalDbName,
      storageStrategy,
      "internal",
      "main"
    );

    // Local WASM database
    const local = await createSurrealDBWasm(
      localDbName,
      storageStrategy,
      namespace,
      database
    );

    // Optional remote HTTP client
    let remote: Surreal | undefined;
    if (remoteUrl) {
      remote = new Surreal({ engines: createRemoteEngines() });
      await remote.connect(remoteUrl);

      if (namespace || database) {
        await remote.use({
          namespace: namespace,
          database: database,
        });
      }
    }

    this.connections = { local, internal, remote };

    // Initialize WAL manager
    this.walManager = new WALManager(internal);
    await this.walManager.init();

    // Provision local schema from schemaSurql
    const provisioner = new SchemaProvisioner(
      internal,
      local,
      namespace,
      database,
      schemaSurql
    );
    await provisioner.provision();

    if (remote) {
      this.syncer = new Syncer(local, remote);
      await this.syncer.init();
    }
  }

  /**
   * Store auth tokens in internal database
   */
  private async storeAuthTokens(
    localToken: string,
    remoteToken?: string
  ): Promise<void> {
    if (!this.connections?.internal)
      throw new Error("SyncedDb not initialized");
    try {
      // Delete existing token first
      await this.connections.internal.query(`DELETE auth_token:current`);
      // Create new token record
      await this.connections.internal.query(
        `CREATE auth_token:current SET local_token = $local_token, remote_token = $remote_token, created_at = time::now()`,
        { local_token: localToken, remote_token: remoteToken }
      );
    } catch (error) {
      console.error("Failed to store auth tokens:", error);
      throw error;
    }
  }

  /**
   * Retrieve auth tokens from internal database
   */
  private async getStoredAuthTokens(): Promise<{
    local: string | null;
    remote: string | null;
  }> {
    if (!this.connections?.internal)
      throw new Error("SyncedDb not initialized");
    try {
      const [result] = await this.connections.internal
        .query(`SELECT local_token, remote_token FROM auth_token:current`)
        .collect<[{ local_token?: string; remote_token?: string }[]]>();

      return {
        local: result?.[0]?.local_token ?? null,
        remote: result?.[0]?.remote_token ?? null,
      };
    } catch (error) {
      console.error("Failed to retrieve auth tokens:", error);
      return { local: null, remote: null };
    }
  }

  /**
   * Remove auth tokens from internal database
   */
  private async removeStoredAuthTokens(): Promise<void> {
    if (!this.connections?.internal)
      throw new Error("SyncedDb not initialized");
    try {
      await this.connections.internal.query(`DELETE auth_token:current`);
    } catch (error) {
      console.error("Failed to remove auth tokens:", error);
    }
  }

  /**
   * Get current user as observable
   */
  getCurrentUser<T extends TableNames<Schema>>(): {
    value: TableModel<GetTable<Schema, T>> | null;
  } {
    return this.currentUser;
  }

  /**
   * Check authentication status and restore session
   */
  async checkAuth<T extends TableNames<Schema>>(
    userTable: T
  ): Promise<TableModel<GetTable<Schema, T>> | null> {
    try {
      const tokens = await this.getStoredAuthTokens();
      if (!tokens.local) {
        this.currentUser.value = null;
        return null;
      }

      // Authenticate local database with stored token
      await this.connections!.local.authenticate(tokens.local);

      // Authenticate remote database if configured
      if (this.connections?.remote && tokens.remote) {
        try {
          await this.connections.remote.authenticate(tokens.remote);
        } catch (error) {
          console.warn("[SyncedDb] Remote authentication failed:", error);
        }
      }

      // Query authenticated user info
      const [users] = await this.queryLocal<TableModel<GetTable<Schema, T>>>(
        `SELECT * FROM $auth`
      ).collect();

      if (users && users.length > 0) {
        const wrappedUser = wrapModelIdWithRef(users[0]);
        this.currentUser.value = wrappedUser;
        return wrappedUser;
      }

      await this.removeStoredAuthTokens();
      this.currentUser.value = null;
      return null;
    } catch (error) {
      console.error("[SyncedDb] Auth check failed:", error);
      await this.removeStoredAuthTokens();
      this.currentUser.value = null;
      return null;
    }
  }

  /**
   * Common authentication flow: authenticate remote, sync user, authenticate local
   */
  private async authenticateRemoteAndSyncUser<T extends TableNames<Schema>>(
    auth: AnyAuth,
    isSignUp: boolean = false
  ): Promise<{
    remoteResponse: AuthResponse;
    localResponse: AuthResponse;
    user: TableModel<GetTable<Schema, T>>;
  }> {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");
    if (!this.connections?.remote)
      throw new Error("Remote database is not configured");

    // Prepare auth for remote (add namespace/database for record access)
    const remoteAuth = { ...auth } as any;
    if (
      "access" in remoteAuth &&
      !remoteAuth.namespace &&
      !remoteAuth.database
    ) {
      remoteAuth.namespace = this.config.namespace;
      remoteAuth.database = this.config.database;
    }

    // Authenticate with remote (sign in or sign up)
    const remoteAuthResponse = isSignUp
      ? await this.connections.remote.signup(remoteAuth)
      : await this.connections.remote.signin(remoteAuth);

    if (!remoteAuthResponse?.token) {
      throw new Error(
        `${
          isSignUp ? "Sign-up" : "Sign-in"
        } failed: No token returned from remote`
      );
    }

    // Query user from remote database
    const [remoteUsers] = await this.connections.remote
      .query(`SELECT * FROM $auth`)
      .collect<[TableModel<GetTable<Schema, T>>[]]>();

    if (!remoteUsers || remoteUsers.length === 0) {
      throw new Error(
        `${isSignUp ? "Sign-up" : "Sign-in"} failed: User not found on remote`
      );
    }

    const remoteUser = remoteUsers[0];
    if (!remoteUser) {
      throw new Error(
        `${isSignUp ? "Sign-up" : "Sign-in"} failed: Invalid user data`
      );
    }

    // Sync user to local database (upsert)
    // First, check if user exists locally
    // const [existingUsers] = await this.connections.local
    //   .query(`SELECT * FROM $id`, { id: remoteUser.id })
    //   .collect<[ModelPayload<Schema[T]>[]]>();

    // if (!existingUsers || existingUsers.length === 0) {
    //   // User doesn't exist locally, create it with the remote user data (including hashed password)
    //   await this.connections.local.query(`CREATE $id CONTENT $user`, {
    //     id: remoteUser.id,
    //     user: remoteUser,
    //   });
    // } else {
    //   // User exists, update it
    //   await this.connections.local.query(`UPDATE $id CONTENT $user`, {
    //     id: remoteUser.id,
    //     user: remoteUser,
    //   });
    // }

    // Sign in to local database - this will work because:
    // 1. For sign-up: the user was just created on remote with the password, so the same password works locally
    // 2. For sign-in: the user now exists locally with the same hashed password from remote
    const localAuthResponse = await this.connections.local.signin({
      access: (auth as any).access,
      variables: (auth as any).variables,
    });

    return {
      remoteResponse: remoteAuthResponse,
      localResponse: localAuthResponse,
      user: remoteUser,
    };
  }

  async signIn<T extends TableNames<Schema>>(
    auth: AnyAuth
  ): Promise<AuthResponse> {
    const { remoteResponse, localResponse, user } =
      await this.authenticateRemoteAndSyncUser<T>(auth, false);

    await this.storeAuthTokens(localResponse.token, remoteResponse.token);
    this.currentUser.value = wrapModelIdWithRef(user);

    return remoteResponse;
  }

  async signUp<T extends TableNames<Schema>>(
    auth: AccessRecordAuth
  ): Promise<AuthResponse> {
    const { remoteResponse, localResponse, user } =
      await this.authenticateRemoteAndSyncUser<T>(auth, true);

    await this.storeAuthTokens(localResponse.token, remoteResponse.token);
    this.currentUser.value = wrapModelIdWithRef(user);

    return remoteResponse;
  }

  async signOut(): Promise<void> {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");

    try {
      // Invalidate local session
      await this.connections.local.invalidate();

      // Invalidate remote session if configured
      if (this.connections.remote) {
        await this.connections.remote.invalidate();
      }
    } catch (error) {
      console.error("Sign out failed:", error);
    } finally {
      // Clear stored tokens and user
      await this.removeStoredAuthTokens();
      this.currentUser.value = null;
    }
  }

  async authenticate(token: string): Promise<void> {
    if (!this.connections?.remote)
      throw new Error("Remote database is not configured");
    try {
      await this.connections.local.authenticate(token);
    } catch (error) {
      console.error("Local authentication failed:", error);
      throw error;
    }
    try {
      await this.connections.remote.authenticate(token);
    } catch (error) {
      console.error("Remote authentication failed:", error);
      throw error;
    }
  }

  getLocal(): Surreal {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");
    return this.connections.local;
  }

  getRemote(): Surreal | undefined {
    return this.connections?.remote;
  }

  /**
   * Sync all pending WAL operations to the remote database
   * Returns the sync result with any failed operations
   */
  async syncWAL(): Promise<{ success: boolean; failedOperations: any[] }> {
    if (!this.walManager) {
      throw new Error("WAL manager not initialized");
    }
    if (!this.connections?.remote) {
      throw new Error("Remote database is not configured");
    }

    return await this.walManager.syncToRemote(
      this.connections.remote,
      this.connections.local
    );
  }

  /**
   * Get pending WAL operations count
   */
  async getPendingWALCount(): Promise<number> {
    if (!this.walManager) {
      throw new Error("WAL manager not initialized");
    }
    const operations = await this.walManager.getPendingOperations();
    return operations.length;
  }

  getSyncer(): Syncer | null {
    return this.syncer;
  }

  _query<T>(
    db: Surreal,
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const res = db.query(sql, vars as any);
    const oCollect = res.collect.bind(res);
    return Object.assign(res, {
      collect: () => oCollect<[ModelPayload<T>[]]>(),
    });
  }

  /**
   * Convert string IDs to RecordId objects for reference fields
   */
  private convertReferenceFields(tableName: string, data: any): any {
    if (!this.config.schema) return data;

    // Find relationships for this table from the schema const
    const tableRelationships = this.config.schema.relationships.filter(
      (rel: { from: string; field: string; to: string; cardinality: string }) =>
        rel.from === tableName
    );

    if (tableRelationships.length === 0) return data;

    const converted = { ...data };

    // Iterate over relationships for this table
    for (const rel of tableRelationships) {
      const fieldValue = converted[rel.field];

      // Skip if field is not present or already a RecordId
      if (fieldValue === undefined || fieldValue === null) continue;
      if (fieldValue instanceof RecordId) continue;

      // Convert string ID to RecordId
      if (typeof fieldValue === "string" && fieldValue.includes(":")) {
        const [table, id] = fieldValue.split(":", 2);
        converted[rel.field] = new RecordId(table, id);
      }
    }

    return converted;
  }

  _create<T extends GenericModel>(
    db: Surreal,
    tableName: string,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["insert"]> {
    const table = new Table(tableName);

    // Convert reference fields for single or multiple records
    const convertedData = Array.isArray(data)
      ? data.map((item) => this.convertReferenceFields(tableName, item))
      : this.convertReferenceFields(tableName, data);

    console.log("[SyncedDb._create] Creating records", convertedData);
    return db.insert(table, convertedData);
  }

  _update<T extends Record<string, unknown> = Record<string, unknown>>(
    db: Surreal,
    recordId: RecordId,
    data: Partial<T>
  ): Promise<any> {
    return db.update<T>(recordId).merge(data);
  }

  _delete<T extends Record<string, unknown> = Record<string, unknown>>(
    db: Surreal,
    table: Table | string
  ): ReturnType<Surreal["delete"]> {
    const tableObj = typeof table === "string" ? new Table(table) : table;
    return db.delete<T>(tableObj);
  }

  queryLocal<T>(sql: string, vars?: Record<string, unknown>): QueryResponse<T> {
    return this._query<T>(this.getLocal(), sql, vars);
  }

  queryRemote<T>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._query<T>(db, sql, vars);
  }

  async createLocal<T extends GenericModel>(
    table: Table | string,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): Promise<any> {
    const tableName =
      typeof table === "string" ? table : table.name || table.toString();

    // Execute on local DB
    const result = await this._create<T>(this.getLocal(), tableName, data);

    // Log to WAL for sync (use structuredClone to avoid proxy issues)
    if (this.walManager) {
      await this.walManager.logCreate(tableName, structuredClone(data));
    }

    return result;
  }

  createRemote<T extends GenericModel>(
    table: Table | string,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["create"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    const tableName =
      typeof table === "string" ? table : table.name || table.toString();
    return this._create<T>(db, tableName, data);
  }

  async updateLocal<
    T extends Record<string, unknown> = Record<string, unknown>
  >(recordId: RecordId, data: Partial<T>): Promise<any> {
    // Get current data for rollback
    let rollbackData: any = null;
    try {
      const current = await this.getLocal().select<T>(recordId);
      rollbackData = structuredClone(current);
    } catch (error) {
      console.warn("[WAL] Could not fetch current data for rollback", error);
    }

    // Execute on local DB
    const result = await this._update<T>(this.getLocal(), recordId, data);

    // Log to WAL for sync (use structuredClone to avoid proxy issues)
    if (this.walManager) {
      const tableName = recordId.toString().split(":")[0];
      await this.walManager.logUpdate(
        tableName,
        recordId,
        structuredClone(data),
        rollbackData
      );
    }

    return result;
  }

  updateRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._update<T>(db, recordId, data);
  }

  async deleteLocal<
    T extends Record<string, unknown> = Record<string, unknown>
  >(recordId: RecordId): Promise<any> {
    // Get current data for rollback
    let rollbackData: any = null;
    try {
      const current = await this.getLocal().select<T>(recordId);
      rollbackData = structuredClone(current);
    } catch (error) {
      console.warn("[WAL] Could not fetch current data for rollback", error);
    }

    // Execute on local DB
    const result = await this.getLocal().delete(recordId);

    // Log to WAL for sync (use structuredClone to avoid proxy issues)
    if (this.walManager) {
      const tableName = recordId.toString().split(":")[0];
      await this.walManager.logDelete(tableName, recordId, rollbackData);
    }

    return result;
  }

  deleteRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table | string
  ): ReturnType<Surreal["delete"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._delete<T>(db, table);
  }
}

export * from "./types";
