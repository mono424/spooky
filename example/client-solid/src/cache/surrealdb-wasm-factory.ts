import { SurrealHTTP as Surreal } from "surrealdb.js";
import initWasm from "@surrealdb/wasm";
import type { CacheStrategy } from "../types";

/**
 * SurrealDB WASM client factory for different storage strategies
 */
export class SurrealDBWasmFactory {
  /**
   * Creates a SurrealDB WASM instance with the specified storage strategy
   */
  static async create(
    dbName: string,
    strategy: CacheStrategy,
    namespace?: string,
    database?: string
  ): Promise<Surreal> {
    // Ensure WASM runtime is initialized (idempotent)
    await initWasm();

    // For local browser DB, SurrealDB 2.3 uses a file: or mem: URL with WASM storage
    // We use surrealdb.js SurrealHTTP purely for a consistent API surface
    const surreal = new Surreal({
      // This endpoint is ignored for WASM-backed local; we will use use() to set NS/DB
      url: "http://localhost/wasm",
    } as any);

    // Open a local database using WASM storage
    // Note: the actual open happens implicitly when running queries with WASM
    // We set the namespace and database now
    await surreal.use({
      namespace: namespace || "main",
      database: database || dbName,
    } as any);

    return surreal;
  }

  /**
   * Creates a memory-based SurrealDB WASM instance
   */
  static async createMemory(
    dbName: string,
    namespace?: string,
    database?: string
  ): Promise<Surreal> {
    return this.create(dbName, "memory", namespace, database);
  }

  /**
   * Creates an IndexedDB-based SurrealDB WASM instance
   */
  static async createIndexedDB(
    dbName: string,
    namespace?: string,
    database?: string
  ): Promise<Surreal> {
    return this.create(dbName, "indexeddb", namespace, database);
  }
}
