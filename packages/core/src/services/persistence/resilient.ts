import type { Logger } from 'pino';
import type { PersistenceClient } from '../../types';

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
      // Best-effort cleanup of the corrupt key; if removal also fails, the key
      // just gets dropped again on the next failed read, so the failure is safe.
      await this.inner.remove(key).catch((removeErr) => {
        this.logger.debug(
          { key, error: removeErr, Category: 'sp00ky-client::ResilientPersistenceClient::get' },
          'Failed to drop corrupt persistence key'
        );
      });
      return null;
    }
  }

  remove(key: string): Promise<void> {
    return this.inner.remove(key);
  }
}
