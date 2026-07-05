import type { Sp00kyConfig } from '../../types';
import type { Logger } from '../logger/index';
import type { LocalEngineChoice, LocalStore } from './cache-engine';
import { SurrealCacheEngine } from './surreal-cache-engine';
import { SqliteCacheEngine } from './sqlite-cache-engine';

/**
 * Build the local cache engine for the given `localEngine` config choice.
 *
 * - `'surrealdb'` / unset → {@link SurrealCacheEngine} (subclass of
 *   `LocalDatabaseService`; the historical behavior, verbatim).
 * - `'sqlite'` → {@link SqliteCacheEngine} (SQLite-WASM Worker + OPFS). Backs
 *   `this.local` through the SurrealQL-vocabulary shim + verb surface.
 * - a custom object → used as-is (must satisfy {@link LocalStore}).
 */
export function createLocalEngine(
  choice: LocalEngineChoice | undefined,
  config: Sp00kyConfig<any>['database'],
  logger: Logger
): LocalStore {
  if (choice === undefined || choice === 'surrealdb') {
    return new SurrealCacheEngine(config, logger);
  }
  if (choice === 'sqlite') {
    // Mirror the SurrealDB engine's `store` semantics: 'memory' → in-memory
    // SQLite (transient), 'indexeddb' → OPFS-backed (durable).
    const useOpfs = (config.store ?? 'memory') !== 'memory';
    return new SqliteCacheEngine(config, logger, { useOpfs });
  }
  // Custom engine instance — trust it to satisfy LocalStore.
  return choice as unknown as LocalStore;
}
