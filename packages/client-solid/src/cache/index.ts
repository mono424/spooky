export { SurrealDBWasmFactory } from './surrealdb-wasm-factory';

import { Surreal } from 'surrealdb';
import type { CacheStrategy } from '../types';

/**
 * Creates a SurrealDB WASM instance with the specified storage strategy
 */
export async function createSurrealDBWasm(
  dbName: string,
  strategy: CacheStrategy,
  namespace?: string,
  database?: string
): Promise<Surreal> {
  const { SurrealDBWasmFactory } = await import('./surrealdb-wasm-factory');
  return SurrealDBWasmFactory.create(dbName, strategy, namespace, database);
}

/**
 * Creates a memory-based SurrealDB WASM instance
 */
export async function createMemoryDB(
  dbName: string,
  namespace?: string,
  database?: string
): Promise<Surreal> {
  const { SurrealDBWasmFactory } = await import('./surrealdb-wasm-factory');
  return SurrealDBWasmFactory.createMemory(dbName, namespace, database);
}

/**
 * Creates an IndexedDB-based SurrealDB WASM instance
 */
export async function createIndexedDBDatabase(
  dbName: string,
  namespace?: string,
  database?: string
): Promise<Surreal> {
  const { SurrealDBWasmFactory } = await import('./surrealdb-wasm-factory');
  return SurrealDBWasmFactory.createIndexedDB(dbName, namespace, database);
}
