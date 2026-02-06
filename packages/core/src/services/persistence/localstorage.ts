import { Logger } from 'pino';
import { PersistenceClient } from '../../types';

export class LocalStoragePersistenceClient implements PersistenceClient {
  private logger: Logger;

  constructor(logger: Logger) {
    this.logger = logger.child({ service: 'PersistenceClient:LocalStorage' });
  }

  set<T>(key: string, value: T): Promise<void> {
    localStorage.setItem(key, JSON.stringify(value));
    return Promise.resolve();
  }

  get<T>(key: string): Promise<T | null> {
    const value = localStorage.getItem(key);
    if (!value) return Promise.resolve(null);
    return Promise.resolve(JSON.parse(value));
  }

  remove(key: string): Promise<void> {
    localStorage.removeItem(key);
    return Promise.resolve();
  }
}
