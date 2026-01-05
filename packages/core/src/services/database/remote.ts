import {
  applyDiagnostics,
  createRemoteEngines,
  Diagnostic,
  Surreal,
  SurrealTransaction,
} from 'surrealdb';
import { SpookyConfig } from '../../types.js';
import { Logger } from '../logger.js';
import { AbstractDatabaseService } from './database.js';

const printDiagnostic = ({ key, type, phase, ...other }: Diagnostic) => {
  if (phase === 'progress' || phase === 'after') {
    console.log(`[SurrealDB:REMOTE] [${key}] ${type}:${phase}\n${JSON.stringify(other, null, 2)}`);
  }
};

export class RemoteDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database'], logger: Logger) {
    super(
      new Surreal({ engines: applyDiagnostics(createRemoteEngines(), printDiagnostic) }),
      logger
    );
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  async connect(): Promise<void> {
    const { endpoint, token, namespace, database } = this.getConfig();
    if (endpoint) {
      this.logger.info({ endpoint, namespace, database }, 'Connecting to remote database');
      try {
        await this.client.connect(endpoint);
        await this.client.use({
          namespace,
          database,
        });

        if (token) {
          this.logger.debug('Authenticating with token');
          await this.client.authenticate(token);
        }
        this.logger.info('Connected to remote database');
      } catch (err) {
        this.logger.error({ err }, 'Failed to connect to remote database');
        throw err;
      }
    } else {
      this.logger.warn('No endpoint configured for remote database');
    }
  }

  async signin(params: any): Promise<any> {
    return this.client.signin(params);
  }

  async signup(params: any): Promise<any> {
    return this.client.signup(params);
  }

  async authenticate(token: string): Promise<any> {
    return this.client.authenticate(token);
  }

  async invalidate(): Promise<void> {
    return this.client.invalidate();
  }
}
