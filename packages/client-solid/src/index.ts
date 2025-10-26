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
import { QueryNamespace, TableQueries, ReactiveQueryResult } from "./lib/table-queries";
import { Syncer } from "./lib/syncer";
import { proxy } from "valtio";
export type { RecordResult } from "surrealdb";

export { RecordId } from "surrealdb";
export type {
  Model,
  GenericModel,
  GenericSchema,
  ModelPayload,
} from "./lib/models";
export { ReactiveQueryResult } from "./lib/table-queries";

export type QueryResponse<T extends GenericModel> = Omit<
  ReturnType<Surreal["query"]>,
  "collect"
> & {
  collect: () => Promise<[ModelPayload<T>[]]>;
};

const AUTH_TOKEN_KEY = "auth_token";

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
    console.log("[SyncedDb] Tables", this.tables);
  }

  logDatabase(db: Surreal) {
    console.log(`Database[cache] version: ${db.version}`);
    console.log(`Database[cache] config: ${db.status}`);
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
   * Store auth token in internal database
   */
  private async storeAuthToken(token: string): Promise<void> {
    if (!this.connections?.internal) throw new Error("SyncedDb not initialized");
    try {
      await this.connections.internal.query(
        `CREATE auth_token:current SET token = $token, created_at = time::now()`,
        { token }
      );
    } catch (error) {
      console.error("Failed to store auth token:", error);
      throw error;
    }
  }

  /**
   * Retrieve auth token from internal database
   */
  private async getStoredAuthToken(): Promise<string | null> {
    if (!this.connections?.internal) throw new Error("SyncedDb not initialized");
    try {
      const [result] = await this.connections.internal
        .query(`SELECT token FROM auth_token:current`)
        .collect<[{ token: string }[]]>();

      return result?.[0]?.token ?? null;
    } catch (error) {
      console.error("Failed to retrieve auth token:", error);
      return null;
    }
  }

  /**
   * Remove auth token from internal database
   */
  private async removeStoredAuthToken(): Promise<void> {
    if (!this.connections?.internal) throw new Error("SyncedDb not initialized");
    try {
      await this.connections.internal.query(`DELETE auth_token:current`);
    } catch (error) {
      console.error("Failed to remove auth token:", error);
    }
  }

  /**
   * Get current user as observable
   */
  getCurrentUser<T extends keyof Schema>(): { value: ModelPayload<Schema[T]> | null } {
    return this.currentUser;
  }

  /**
   * Check authentication status and restore session
   */
  async checkAuth<T extends keyof Schema>(
    userTable: T
  ): Promise<ModelPayload<Schema[T]> | null> {
    console.log("[SyncedDb] Checking authentication");
    try {
      const token = await this.getStoredAuthToken();
      if (!token) {
        console.log("[SyncedDb] No stored token found");
        this.currentUser.value = null;
        return null;
      }

      // Authenticate with stored token
      console.log("[SyncedDb] Authenticating with token");
      await this.authenticate(token);

      console.log("[SyncedDb] Querying authenticated user info");
      // Query authenticated user info
      const [users] = await this.queryLocal<Schema[T]>(
        `SELECT * FROM $auth`
      ).collect();

      console.log("[SyncedDb] Authenticated user info", users);
      if (users && users.length > 0) {
        this.currentUser.value = users[0];
        return users[0];
      } else {
        await this.removeStoredAuthToken();
        this.currentUser.value = null;
        return null;
      }
    } catch (error) {
      console.error("[SyncedDb] Auth check failed:", error);
      await this.removeStoredAuthToken();
      this.currentUser.value = null;
      return null;
    }
  }

  async signIn<T extends keyof Schema>(
    auth: AnyAuth,
    userTable: T
  ): Promise<AuthResponse> {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");

    // Sign in to local database
    let authResponse: AuthResponse;
    try {
      authResponse = await this.connections.local.signin(auth);
    } catch (error) {
      console.error("[SyncedDb] Local sign-in failed:", error);
      throw error;
    }

    if (!authResponse?.token) {
      throw new Error("Sign-in failed: No token returned");
    }

    // Sign in to remote database if configured
    if (this.connections.remote) {
      try {
        // For record access, ensure namespace and database are included
        const remoteAuth = { ...auth } as any;
        if ('access' in remoteAuth && !remoteAuth.namespace && !remoteAuth.database) {
          remoteAuth.namespace = this.config.namespace;
          remoteAuth.database = this.config.database;
        }
        await this.connections.remote.signin(remoteAuth);
        console.log("[SyncedDb] Remote sign-in successful");
      } catch (error) {
        console.error("[SyncedDb] Remote sign-in failed:", error);
        throw error;
      }
    }

    // Authenticate both databases with the token
    try {
      await this.connections.local.authenticate(authResponse.token);
      if (this.connections.remote) {
        await this.connections.remote.authenticate(authResponse.token);
      }
    } catch (error) {
      console.error("[SyncedDb] Authentication failed:", error);
      throw error;
    }

    // Store token in internal database
    await this.storeAuthToken(authResponse.token);

    // Query and set current user
    const [users] = await this.queryLocal<Schema[T]>(
      `SELECT * FROM $auth`
    ).collect();
    if (users && users.length > 0) {
      this.currentUser.value = users[0];
    }

    return authResponse;
  }

  async signUp<T extends keyof Schema>(
    auth: AccessRecordAuth,
    userTable: T
  ): Promise<AuthResponse> {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");

    // Sign up to local database
    let authResponse: AuthResponse;
    try {
      authResponse = await this.connections.local.signup(auth);
    } catch (error) {
      console.error("Local sign-up failed:", error);
      throw error;
    }

    if (!authResponse?.token) {
      throw new Error("Sign-up failed: No token returned");
    }

    // Sign up to remote database if configured
    if (this.connections.remote) {
      try {
        await this.connections.remote.signup(auth);
      } catch (error) {
        console.error("Remote sign-up failed:", error);
        throw error;
      }
    }

    // Authenticate both databases with the token
    try {
      await this.connections.local.authenticate(authResponse.token);
      if (this.connections.remote) {
        await this.connections.remote.authenticate(authResponse.token);
      }
    } catch (error) {
      console.error("Authentication failed:", error);
      throw error;
    }

    // Store token in internal database
    await this.storeAuthToken(authResponse.token);

    // Query and set current user
    const [users] = await this.queryLocal<Schema[T]>(
      `SELECT * FROM $auth`
    ).collect();
    if (users && users.length > 0) {
      this.currentUser.value = users[0];
    }

    return authResponse;
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
      // Clear stored token and user
      await this.removeStoredAuthToken();
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
    table: Table,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["insert"]> {
    console.log("createLocal", table, data);
    return db.insert<ModelPayload<T>>(table, data);
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
    table: Table
  ): ReturnType<Surreal["delete"]> {
    return db.delete<T>(table);
  }

  queryLocal<T extends GenericModel>(
    sql: string,
    vars?: Record<string, unknown>
  ): QueryResponse<T> {
    const db = this.getLocal();
    return this._query<T>(db, sql, vars);
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
    thing: Table,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["create"]> {
    const db = this.getLocal();
    return this._create<T>(db, thing, data);
  }

  createRemote<T extends GenericModel>(
    table: Table,
    data: Values<ModelPayload<T>> | Values<ModelPayload<T>>[]
  ): ReturnType<Surreal["create"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._create<T>(db, table, data);
  }

  updateLocal<T extends Record<string, unknown> = Record<string, unknown>>(
    recordId: RecordId,
    data: Partial<T>
  ): ReturnType<Surreal["update"]> {
    const db = this.getLocal();
    return this._update<T>(db, recordId, data);
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
    table: Table
  ): ReturnType<Surreal["delete"]> {
    const db = this.getLocal();
    return this._delete<T>(db, table);
  }

  deleteRemote<T extends Record<string, unknown> = Record<string, unknown>>(
    table: Table
  ): ReturnType<Surreal["delete"]> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    return this._delete<T>(db, table);
  }
}

export * from "./types";
