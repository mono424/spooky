import { applyDiagnostics, DateTime, Diagnostic, RecordId, Surreal } from 'surrealdb';
import { createWasmWorkerEngines } from '@surrealdb/wasm';
import { SpookyConfig } from '../../types.js';
import { Logger } from '../logger/index.js';
import { AbstractDatabaseService } from './database.js';
import { createDatabaseEventSystem, DatabaseEventTypes } from './events/index.js';
import { encodeRecordId, parseRecordIdString, surql } from '../../utils/index.js';

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];
  protected eventType = DatabaseEventTypes.LocalQuery;

  constructor(config: SpookyConfig<any>['database'], logger: Logger) {
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
                  Category: 'spooky-client::LocalDatabaseService::diagnostics',
                },
                `Local SurrealDB diagnostics captured ${key} ${type}:${phase}`
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

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  async connect(): Promise<void> {
    const { namespace, database } = this.getConfig();
    this.logger.info(
      { namespace, database, Category: 'spooky-client::LocalDatabaseService::connect' },
      'Connecting to local database'
    );
    try {
      const store = this.getConfig().store ?? 'memory';
      const storeUrl = store === 'memory' ? 'mem://' : 'indxdb://spooky';
      this.logger.debug(
        { storeUrl, Category: 'spooky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] Calling client.connect'
      );
      await this.client.connect(storeUrl, {});
      this.logger.debug(
        { namespace, database, Category: 'spooky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] client.connect returned. Calling client.use'
      );

      await this.client.use({
        namespace,
        database,
      });
      this.logger.debug(
        { Category: 'spooky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] client.use returned'
      );

      this.logger.info(
        { Category: 'spooky-client::LocalDatabaseService::connect' },
        'Connected to local database'
      );
    } catch (err) {
      this.logger.error(
        { err, Category: 'spooky-client::LocalDatabaseService::connect' },
        'Failed to connect to local database'
      );
      throw err;
    }
  }
}
