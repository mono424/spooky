import type { Diagnostic} from 'surrealdb';
import { applyDiagnostics, DateTime, RecordId, Surreal } from 'surrealdb';
import { createWasmWorkerEngines } from '@surrealdb/wasm';
import type { Sp00kyConfig } from '../../types';
import type { Logger } from '../logger/index';
import { AbstractDatabaseService } from './database';
import { createDatabaseEventSystem, DatabaseEventTypes } from './events/index';
import { encodeRecordId } from '../../utils/index';

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: Sp00kyConfig<any>['database'];
  protected eventType = DatabaseEventTypes.LocalQuery;

  constructor(config: Sp00kyConfig<any>['database'], logger: Logger) {
    const events = createDatabaseEventSystem();
    super(
      new Surreal({
        codecOptions: {
          valueDecodeVisitor(value) {
            if (value instanceof RecordId) {
              return encodeRecordId(value);
            }

            if (value instanceof DateTime) {
              return value.toDate();
            }

            return value;
          },
        },
        engines: applyDiagnostics(
          createWasmWorkerEngines(),
          ({ key, type, phase, ...other }: Diagnostic) => {
            if (phase === 'progress' || phase === 'after') {
              logger.trace(
                {
                  ...other,
                  key,
                  type,
                  phase,
                  service: 'surrealdb:local',
                  Category: 'sp00ky-client::LocalDatabaseService::diagnostics',
                },
                `Local SurrealDB diagnostics captured ${type}:${phase}`
              );
            }
          }
        ),
      }),
      logger,
      events
    );
    this.config = config;
  }

  getConfig(): Sp00kyConfig<any>['database'] {
    return this.config;
  }

  async connect(): Promise<void> {
    const { namespace, database } = this.getConfig();
    const store = this.getConfig().store ?? 'memory';
    const storeUrl = store === 'memory' ? 'mem://' : 'indxdb://sp00ky';
    this.logger.info(
      { namespace, database, storeUrl, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      'Connecting to local database'
    );

    this.registerUnloadClose();

    try {
      await this.openStore(storeUrl, namespace, database);
      this.logger.info(
        { Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Connected to local database'
      );
      return;
    } catch (err) {
      // A persistent (IndexedDB) local store can fail to open if it was left
      // corrupt or version-incompatible by a prior session/crash/engine bump.
      // The local store is only a cache (everything re-syncs from the server),
      // so recover by dropping it and reconnecting rather than bricking startup.
      if (store === 'memory' || !isLocalStoreOpenError(err)) {
        this.logger.error(
          { err, Category: 'sp00ky-client::LocalDatabaseService::connect' },
          'Failed to connect to local database'
        );
        throw err;
      }
      this.logger.warn(
        { err, Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Local IndexedDB store failed to open; retrying before clearing'
      );
    }

    // Tier 1 — RETRY the SAME store WITHOUT dropping. The idb open/`use` failure
    // is often transient (a not-yet-released handle from the previous page, or a
    // first-open WAL-recovery race), not real corruption. Closing and reopening
    // frequently succeeds — and crucially PRESERVES the cache, so a warm load
    // stays warm. Dropping the store every time (the old behavior) silently wiped
    // the cache on every reload, making warm loads as slow as cold ones.
    for (let attempt = 1; attempt <= 2; attempt++) {
      try {
        await this.client.close();
      } catch {
        /* ignore */
      }
      await delay(150 * attempt);
      try {
        await this.openStore(storeUrl, namespace, database);
        this.logger.info(
          { attempt, Category: 'sp00ky-client::LocalDatabaseService::connect' },
          'Connected to local database on retry (cache preserved)'
        );
        return;
      } catch (retryErr) {
        this.logger.warn(
          { err: retryErr, attempt, Category: 'sp00ky-client::LocalDatabaseService::connect' },
          'Local store retry failed'
        );
      }
    }

    // Tier 2 — the store is genuinely unopenable; drop it and reconnect fresh.
    // This loses the cache (re-syncs from the server), so it's the last resort
    // before in-memory.
    try {
      await this.client.close();
    } catch {
      /* ignore — closing a half-open connection is best-effort */
    }
    await dropLocalIndexedDbStores(this.logger);

    try {
      await this.openStore(storeUrl, namespace, database);
      this.logger.info(
        { Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Reconnected to local database after clearing the corrupt store'
      );
    } catch (retryErr) {
      // Last resort: run in-memory so the app still loads. No local persistence
      // this session; the freshly-dropped IndexedDB is recreated cleanly next
      // load, and all data re-syncs from the server regardless.
      this.logger.error(
        { err: retryErr, Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Local store still failing after clear; falling back to in-memory'
      );
      try {
        await this.client.close();
      } catch {
        /* ignore */
      }
      await this.openStore('mem://', namespace, database);
      this.logger.warn(
        { Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Connected to local database (in-memory fallback)'
      );
    }
  }

  private unloadCloseRegistered = false;

  /**
   * Close the local DB on page unload so the SurrealDB-WASM worker releases its
   * IndexedDB connection cleanly. Without this, the previous page's connection
   * lingers; the next load's `client.connect` opens the store but the first
   * write transaction in `client.use` hits an "IndexedDB error" — which then
   * (mis)triggered the corrupt-store recovery and WIPED the cache on every
   * reload, making warm loads as slow as cold ones. `pagehide` is the reliable
   * unload signal (fires on bfcache + normal navigation); `close()` is async but
   * the WASM worker initiates the IndexedDB connection teardown synchronously.
   */
  private registerUnloadClose(): void {
    if (this.unloadCloseRegistered || typeof window === 'undefined') return;
    this.unloadCloseRegistered = true;
    const close = () => {
      try {
        void this.client.close();
      } catch {
        /* best-effort */
      }
    };
    window.addEventListener('pagehide', close);
    window.addEventListener('beforeunload', close);
  }

  private async openStore(storeUrl: string, namespace: string, database: string): Promise<void> {
    this.logger.debug(
      { storeUrl, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      '[LocalDatabaseService] Calling client.connect'
    );
    await this.client.connect(storeUrl, {});
    this.logger.debug(
      { namespace, database, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      '[LocalDatabaseService] client.connect returned. Calling client.use'
    );
    await this.client.use({ namespace, database });
    this.logger.debug(
      { Category: 'sp00ky-client::LocalDatabaseService::connect' },
      '[LocalDatabaseService] client.use returned'
    );
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** True for the SurrealDB-WASM error raised when its IndexedDB-backed key-value
 *  store can't be opened (corrupt / version-incompatible / blocked). Exported
 *  for unit testing the error-message match. */
export function isLocalStoreOpenError(err: unknown): boolean {
  const msg = (err instanceof Error ? err.message : String(err)).toLowerCase();
  return (
    msg.includes('indexeddb') ||
    msg.includes('idb error') ||
    msg.includes('key-value store')
  );
}

/** Best-effort delete of this client's IndexedDB store(s). The persistent local
 *  DB lives at `indxdb://sp00ky`; SurrealDB-WASM backs it with one or more
 *  IndexedDB databases whose names include `sp00ky`. Resolves even on
 *  error/blocked so startup can proceed. No-op outside a browser. */
async function dropLocalIndexedDbStores(logger: Logger): Promise<void> {
  if (typeof indexedDB === 'undefined') return;
  const remove = (name: string): Promise<void> =>
    new Promise((resolve) => {
      try {
        const req = indexedDB.deleteDatabase(name);
        req.onsuccess = () => resolve();
        req.onerror = () => resolve();
        req.onblocked = () => resolve();
      } catch {
        resolve();
      }
    });
  try {
    let names: string[] = [];
    if (typeof indexedDB.databases === 'function') {
      const dbs = await indexedDB.databases();
      names = dbs
        .map((d) => d.name)
        .filter((n): n is string => !!n && n.toLowerCase().includes('sp00ky'));
    }
    // Fall back to the known store name if enumeration is unavailable/empty.
    if (names.length === 0) names = ['sp00ky'];
    await Promise.all(names.map(remove));
    logger.info(
      { names, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      'Cleared local IndexedDB store(s)'
    );
  } catch (e) {
    logger.warn(
      { err: e, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      'Failed to enumerate/clear IndexedDB; proceeding anyway'
    );
  }
}
