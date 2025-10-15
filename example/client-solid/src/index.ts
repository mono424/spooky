import { SurrealHTTP as Surreal } from "surrealdb";
import initWasm from "@surrealdb/wasm";
import { SchemaProvisioner } from "./schema/provisioner";
import { createSurrealDBWasm } from "./cache";
import type { SyncedDbConfig, DbConnection } from "./types";

export class SyncedDb {
  private config: SyncedDbConfig;
  private connections: DbConnection | null = null;

  constructor(config: SyncedDbConfig) {
    this.config = config;
  }

  /**
   * Initialize local WASM DB and optional remote client, then provision local schema
   */
  async init(): Promise<void> {
    await initWasm();

    const {
      localDbName,
      storageStrategy,
      namespace,
      database,
      remoteUrl,
      token,
    } = this.config;

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
      remote = new Surreal({ url: remoteUrl } as any);
      if (namespace || database) {
        await remote.use({
          namespace: namespace || "main",
          database: database || "main",
        } as any);
      }
      if (token) {
        await remote.signin({ token } as any);
      }
    }

    this.connections = { local, remote };

    // Provision local schema from src/db/schema.surql
    const provisioner = new SchemaProvisioner(local);
    await provisioner.provision();
  }

  getLocal(): Surreal {
    if (!this.connections?.local) throw new Error("SyncedDb not initialized");
    return this.connections.local;
  }

  getRemote(): Surreal | undefined {
    return this.connections?.remote;
  }

  async queryLocal<T = unknown>(
    sql: string,
    vars?: Record<string, unknown>
  ): Promise<T> {
    const db = this.getLocal();
    const res = (await db.query(sql, vars as any)) as unknown as T;
    return res;
  }

  async queryRemote<T = unknown>(
    sql: string,
    vars?: Record<string, unknown>
  ): Promise<T> {
    const db = this.getRemote();
    if (!db) throw new Error("Remote database is not configured");
    const res = (await db.query(sql, vars as any)) as unknown as T;
    return res;
  }
}

export * from "./types";
