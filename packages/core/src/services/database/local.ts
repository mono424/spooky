import { applyDiagnostics, DateTime, Diagnostic, RecordId, Surreal } from 'surrealdb';
import { createWasmWorkerEngines } from '@surrealdb/wasm';
import { Sp00kyConfig } from '../../types';
import { Logger } from '../logger/index';
import { AbstractDatabaseService } from './database';
import { createDatabaseEventSystem, DatabaseEventTypes } from './events/index';
import { encodeRecordId, parseRecordIdString, surql } from '../../utils/index';

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
    this.logger.info(
      { namespace, database, Category: 'sp00ky-client::LocalDatabaseService::connect' },
      'Connecting to local database'
    );
    try {
      const store = this.getConfig().store ?? 'memory';
      const storeUrl = store === 'memory' ? 'mem://' : 'indxdb://sp00ky';
      this.logger.debug(
        { storeUrl, Category: 'sp00ky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] Calling client.connect'
      );
      await this.client.connect(storeUrl, {});
      this.logger.debug(
        { namespace, database, Category: 'sp00ky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] client.connect returned. Calling client.use'
      );

      await this.client.use({
        namespace,
        database,
      });
      this.logger.debug(
        { Category: 'sp00ky-client::LocalDatabaseService::connect' },
        '[LocalDatabaseService] client.use returned'
      );

      this.logger.info(
        { Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Connected to local database'
      );
    } catch (err) {
      this.logger.error(
        { err, Category: 'sp00ky-client::LocalDatabaseService::connect' },
        'Failed to connect to local database'
      );
      throw err;
    }
  }
}
