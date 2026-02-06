import {
  applyDiagnostics,
  createRemoteEngines,
  Diagnostic,
  Surreal,
  SurrealTransaction,
} from 'surrealdb';
import { SpookyConfig } from '../../types';
import { Logger } from '../logger/index';
import { AbstractDatabaseService } from './database';
import { createDatabaseEventSystem, DatabaseEventTypes } from './events/index';

export class RemoteDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];
  protected eventType = DatabaseEventTypes.RemoteQuery;

  constructor(config: SpookyConfig<any>['database'], logger: Logger) {
    const events = createDatabaseEventSystem();
    super(
      new Surreal({
        engines: applyDiagnostics(
          createRemoteEngines(),
          ({ key, type, phase, ...other }: Diagnostic) => {
            if (phase === 'progress' || phase === 'after') {
              logger.trace(
                {
                  ...other,
                  key,
                  type,
                  phase,
                  service: 'surrealdb:remote',
                  Category: 'spooky-client::RemoteDatabaseService::diagnostics',
                },
                `Remote SurrealDB diagnostics captured ${type}:${phase}`
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
    const { endpoint, token, namespace, database } = this.getConfig();
    if (endpoint) {
      this.logger.info(
        {
          endpoint,
          namespace,
          database,
          Category: 'spooky-client::RemoteDatabaseService::connect',
        },
        'Connecting to remote database'
      );
      try {
        await this.client.connect(endpoint);
        await this.client.use({
          namespace,
          database,
        });

        if (token) {
          this.logger.debug(
            { Category: 'spooky-client::RemoteDatabaseService::connect' },
            'Authenticating with token'
          );
          await this.client.authenticate(token);
        }
        this.logger.info(
          { Category: 'spooky-client::RemoteDatabaseService::connect' },
          'Connected to remote database'
        );
      } catch (err) {
        this.logger.error(
          { err, Category: 'spooky-client::RemoteDatabaseService::connect' },
          'Failed to connect to remote database'
        );
        throw err;
      }
    } else {
      this.logger.warn(
        { Category: 'spooky-client::RemoteDatabaseService::connect' },
        'No endpoint configured for remote database'
      );
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
