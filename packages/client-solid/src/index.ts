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
import { GenericModel, GenericSchema, ModelPayload } from "./lib/models";
import {
  QueryNamespace,
  TableQueries,
  ReactiveQueryResult,
} from "./lib/table-queries";
import { Syncer } from "./lib/syncer";
import { proxy, ref } from "valtio";
export type { RecordResult } from "surrealdb";

export { RecordId } from "surrealdb";
export type {
  Model,
  GenericModel,
  GenericSchema,
  ModelPayload,
} from "./lib/models";
export { ReactiveQueryResult } from "./lib/table-queries";

export { snapshot } from "valtio";

export type QueryResponse<T extends GenericModel> = Omit<
  ReturnType<Surreal["query"]>,
  "collect"
> & {
  collect: () => Promise<[ModelPayload<T>[]]>;
};

/**
 * Wrap a single model's id with ref() to prevent valtio from proxying RecordId
 */
function wrapModelIdWithRef<T extends GenericModel>(model: T): T {
  if (model && model.id) {
    return { ...model, id: ref(model.id) };
  }
  return model;
}

export class SyncedDb<Schema extends GenericSchema> {
  private config: SyncedDbConfig<Schema>;
  private connections: DbConnection | null = null;
  public readonly query: TableQueries<Schema>;
  private tables: Table[] = [];
  private syncer: Syncer | null = null;
  private currentUser: any = proxy({ value: null });

  constructor(config: SyncedDbConfig<Schema>) {
    this.config = config;
    this.tables = config.tables.map((table) => new Table(table));
    this.query = new QueryNamespace<Schema>(this) as QueryNamespace<Schema> &
      TableQueries<Schema>;
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
      schema,
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

    // Provision local schema from src/db/schema.surql
    const provisioner = new SchemaProvisioner(
      internal,
      local,
      namespace,
      database,
      schema
    );
    await provisioner.provision();

    if (remote) {
      this.syncer = new Syncer(local, remote, this.tables);
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
  getCurrentUser<T extends keyof Schema>(): {
    value: ModelPayload<Schema[T]> | null;
  } {
    return this.currentUser;
  }

  /**
   * Check authentication status and restore session
   */
  async checkAuth<T extends keyof Schema>(
    userTable: T
  ): Promise<ModelPayload<Schema[T]> | null> {
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
      const [users] = await this.queryLocal<Schema[T]>(
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
  private async authenticateRemoteAndSyncUser<T extends keyof Schema>(
    auth: AnyAuth,
    isSignUp: boolean = false
  ): Promise<{
    remoteResponse: AuthResponse;
    localResponse: AuthResponse;
    user: ModelPayload<Schema[T]>;
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
      .collect<[ModelPayload<Schema[T]>[]]>();

    if (!remoteUsers || remoteUsers.length === 0) {
      throw new Error(
        `${isSignUp ? "Sign-up" : "Sign-in"} failed: User not found on remote`
      );
    }

    const remoteUser = remoteUsers[0];

    // Sync user to local database (upsert)
    await this.connections.local.query(`UPDATE $id CONTENT $user`, {
      id: remoteUser.id,
      user: remoteUser,
    });

    // Sign in to local database
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

  async signIn<T extends keyof Schema>(auth: AnyAuth): Promise<AuthResponse> {
    const { remoteResponse, localResponse, user } =
      await this.authenticateRemoteAndSyncUser<T>(auth, false);

    await this.storeAuthTokens(localResponse.token, remoteResponse.token);
    this.currentUser.value = wrapModelIdWithRef(user);

    return remoteResponse;
  }

  async signUp<T extends keyof Schema>(
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

  getSyncer(): Syncer | null {
    return this.syncer;
  }

  _query<T extends GenericModel>(
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

  _create<T extends GenericModel>(
    db: Surreal,
    tableName: string,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["insert"]> {
    const table = new Table(tableName);
    return db.insert(table, data);
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

  queryLocal<T extends GenericModel>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    return this._query<T>(this.getLocal(), sql, vars);
  }

  queryRemote<T extends GenericModel>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._query<T>(db, sql, vars);
  }

  createLocal<T extends GenericModel>(
    table: Table | string,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["create"]> {
    const tableName =
      typeof table === "string" ? table : table.name || table.toString();
    return this._create<T>(this.getLocal(), tableName, data);
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

  updateLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    return this._update<T>(this.getLocal(), recordId, data);
  }

  updateRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._update<T>(db, recordId, data);
  }

  deleteLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table | string
  ): ReturnType<Surreal["delete"]> {
    return this._delete<T>(this.getLocal(), table);
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
