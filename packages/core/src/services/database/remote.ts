import { Surreal, SurrealTransaction } from 'surrealdb';
import { SpookyConfig } from '../../types.js';
import { AbstractDatabaseService } from './database.js';

export class RemoteDatabaseService extends AbstractDatabaseService {
  private config: SpookyConfig<any>['database'];

  constructor(config: SpookyConfig<any>['database']) {
    super(new Surreal());
    this.config = config;
  }

  getConfig(): SpookyConfig<any>['database'] {
    return this.config;
  }

  async connect(): Promise<void> {
    const { endpoint, token, namespace, database } = this.getConfig();
    if (endpoint) {
      await this.client.connect(endpoint);
      await this.client.use({
        namespace,
        database,
      });

      if (token) {
        await this.client.authenticate(token);
      }
    }
  }

  async subscribeLive(
    uuid: string,
    callback: (action: string, result: Record<string, unknown>) => void
  ) {
    try {
      // Correct API for SurrealDB v2+ is .subscribe(uuid)
      // @ts-ignore
      await this.client.subscribe(uuid, (action: string, result: Record<string, unknown>) => {
        console.log('[RemoteService] Live Event:', action, result);
        callback(action, result);
      });
    } catch (e) {
      console.error('Live query subscription error', e);
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
