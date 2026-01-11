import { applyDiagnostics, Diagnostic, Surreal } from 'surrealdb';
import { createWasmWorkerEngines } from '@surrealdb/wasm';
import { SpookyConfig } from '../../types.js';
import { Logger } from '../logger/index.js';
import { AbstractDatabaseService } from './database.js';

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database'], logger: Logger) {
    super(
      new Surreal({
        engines: applyDiagnostics(
          createWasmWorkerEngines(),
          ({ key, type, phase, ...other }: Diagnostic) => {
            if (phase === 'progress' || phase === 'after') {
              logger.info(
                { ...other, key, type, phase, service: 'surrealdb:local' },
                `[SurrealDB:LOCAL] [${key}] ${type}:${phase}`
              );
            }
          }
        ),
      }),
      logger
    );
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  async connect(): Promise<void> {
    const { namespace, database } = this.getConfig();
    this.logger.info({ namespace, database }, 'Connecting to local database');
    try {
      const store = this.getConfig().store ?? 'memory';
      const storeUrl = store === 'memory' ? 'mem://' : 'indxdb://spooky';
      console.log(`[LocalDatabaseService] Calling client.connect(${storeUrl}, {})...`);
      await this.client.connect(storeUrl, {});
      console.log(
        `[LocalDatabaseService] client.connect returned. Calling client.use(${namespace}, ${database})...`
      );

      await this.client.use({
        namespace,
        database,
      });
      console.log('[LocalDatabaseService] client.use returned.');

      this.logger.info('Connected to local database');
    } catch (err) {
      console.error('[LocalDatabaseService] Error during connect:', err);
      this.logger.error({ err }, 'Failed to connect to local database');
      throw err;
    }
  }
}
