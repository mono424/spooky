import type { Diagnostic} from 'surrealdb';
import { Surreal, applyDiagnostics } from 'surrealdb';
import { createWasmEngines } from '@surrealdb/wasm';
import type { CacheStrategy } from '../types';

const printDiagnostic = ({ key, type, phase, ...other }: Diagnostic) => {
  if (phase === 'progress' || phase === 'after') {
    // oxlint-disable-next-line no-console -- intentional diagnostic logging
    console.log(`[SurrealDB_WASM] [${key}] ${type}:${phase}\n${JSON.stringify(other, null, 2)}`);
  }
};

/**
 * SurrealDB WASM client factory for different storage strategies
 */
// oxlint-disable-next-line no-extraneous-class -- factory pattern groups related static methods
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
    // Create Surreal instance with WASM engines
    const surreal = new Surreal({
      engines: applyDiagnostics(createWasmEngines(), printDiagnostic),
    });

    // Connect to the appropriate storage backend
    const connectionUrl = strategy === 'indexeddb' ? `indxdb://${dbName}` : 'mem://';

    await surreal.connect(connectionUrl);

    // Set namespace and database
    await surreal.use({
      namespace: namespace || 'main',
      database: database || dbName,
    });

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
    return this.create(dbName, 'memory', namespace, database);
  }

  /**
   * Creates an IndexedDB-based SurrealDB WASM instance
   */
  static async createIndexedDB(
    dbName: string,
    namespace?: string,
    database?: string
  ): Promise<Surreal> {
    return this.create(dbName, 'indexeddb', namespace, database);
  }
}
