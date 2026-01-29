import { applyDiagnostics, Diagnostic, RecordId, Surreal } from 'surrealdb';
import { createWasmWorkerEngines } from '@surrealdb/wasm';
import { SpookyConfig } from '../../types.js';
import { Logger } from '../logger/index.js';
import { AbstractDatabaseService } from './database.js';
import { createDatabaseEventSystem, DatabaseEventTypes } from './events/index.js';
import { encodeRecordId, parseRecordIdString } from '../../utils/index.js';

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

            return value;
          },
        },
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
      logger,
      events
    );
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  async setKv(key: string, val: any) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      await this.client.query(`CREATE ONLY ${id} SET value = $val`, { val });
    } catch (error) {
      this.logger.error({ error }, 'Failed to set KV');
      throw error;
    }
  }

  async getKv<T>(key: string) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      const [result] = await this.client.query<[T]>(`SELECT value FROM ONLY ${id}`);
      return result;
    } catch (error) {
      this.logger.warn({ error }, 'Failed to get KV');
      throw error;
    }
  }

  async deleteKv(key: string) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      await this.client.query(`DELETE FROM ONLY ${id}`);
    } catch (err) {
      this.logger.info({ err }, 'Failed to delete KV');
    }
  }

  async connect(): Promise<void> {
    const { namespace, database } = this.getConfig();
    this.logger.info({ namespace, database }, 'Connecting to local database');
    try {
      const store = this.getConfig().store ?? 'memory';
      const storeUrl = store === 'memory' ? 'mem://' : 'indxdb://spooky';
      this.logger.debug({ storeUrl }, '[LocalDatabaseService] Calling client.connect');
      await this.client.connect(storeUrl, {});
      this.logger.debug(
        { namespace, database },
        '[LocalDatabaseService] client.connect returned. Calling client.use'
      );

      await this.client.use({
        namespace,
        database,
      });
      this.logger.debug('[LocalDatabaseService] client.use returned');

      this.logger.info('Connected to local database');
    } catch (err) {
      this.logger.error({ err }, '[LocalDatabaseService] Error during connect');
      this.logger.error({ err }, 'Failed to connect to local database');
      throw err;
    }
  }
}
