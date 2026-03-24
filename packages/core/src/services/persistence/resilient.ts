import { Logger } from 'pino';
import { PersistenceClient } from '../../types';

export class ResilientPersistenceClient implements PersistenceClient {
  private logger: Logger;

  constructor(
    private inner: PersistenceClient,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'ResilientPersistenceClient' });
  }

  set<T>(key: string, value: T): Promise<void> {
    return this.inner.set(key, value);
  }

  async get<T>(key: string): Promise<T | null> {
    try {
      return await this.inner.get<T>(key);
    } catch (e) {
      this.logger.warn(
        { key, error: e, Category: 'sp00ky-client::ResilientPersistenceClient::get' },
        'Persistence read failed, dropping key'
      );
      await this.inner.remove(key).catch(() => {});
      return null;
    }
  }

  remove(key: string): Promise<void> {
    return this.inner.remove(key);
  }
}
