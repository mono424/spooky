import { applyDiagnostics, Diagnostic, Surreal } from 'surrealdb';
import { createWasmEngines } from '@surrealdb/wasm';
import { SpookyConfig } from '../../types.js';
import { Logger } from '../logger.js';
import { AbstractDatabaseService } from './database.js';

const printDiagnostic = ({ key, type, phase, ...other }: Diagnostic) => {
  if (phase === 'progress' || phase === 'after') {
    console.log(`[SurrealDB:LOCAL] [${key}] ${type}:${phase}\n${JSON.stringify(other, null, 2)}`);
  }
};

export class LocalDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database'], logger: Logger) {
    super(
      new Surreal({
        engines: applyDiagnostics(createWasmEngines(), printDiagnostic),
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
      await this.client.connect('indxdb://spooky', {});
      await this.client.use({
        namespace,
        database,
      });
      this.logger.info('Connected to local database');
    } catch (err) {
      this.logger.error({ err }, 'Failed to connect to local database');
      throw err;
    }
  }
}
