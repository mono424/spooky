import { PersistenceClient } from '../../types.js';

export class LocalStoragePersistenceClient implements PersistenceClient {
  set(key: string, value: string): Promise<void> {
    localStorage.setItem(key, value);
    return Promise.resolve();
  }

  get(key: string): Promise<string | null> {
    return Promise.resolve(localStorage.getItem(key));
  }

  remove(key: string): Promise<void> {
    localStorage.removeItem(key);
    return Promise.resolve();
  }
}
