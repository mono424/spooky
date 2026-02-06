import { PersistenceClient } from '../../types';
import { parseRecordIdString, surql } from '../../utils/index';
import { Logger } from 'pino';
import { AbstractDatabaseService } from '../database/database';

export class SurrealDBPersistenceClient implements PersistenceClient {
  private logger: Logger;

  constructor(
    private db: AbstractDatabaseService,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'PersistenceClient:SurrealDb' });
  }

  async set<T>(key: string, val: T) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      await this.db.query(surql.seal(surql.upsert('id', 'data')), { id, data: { val } });
    } catch (error) {
      this.logger.error(
        { error, Category: 'spooky-client::SurrealDBPersistenceClient::set' },
        'Failed to set KV'
      );
      throw error;
    }
  }

  async get<T>(key: string) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      const [result] = await this.db.query<[{ val: T }]>(
        surql.seal(surql.selectById('id', ['val'])),
        {
          id,
        }
      );
      if (!result?.val) {
        return null;
      }
      return result.val;
    } catch (error) {
      this.logger.warn(
        { error, Category: 'spooky-client::SurrealDBPersistenceClient::get' },
        'Failed to get KV'
      );
      return null;
    }
  }

  async remove(key: string) {
    try {
      const id = parseRecordIdString(`_spooky_kv:${key}`);
      await this.db.query(surql.seal(surql.delete('id')), { id });
    } catch (err) {
      this.logger.info(
        { err, Category: 'spooky-client::SurrealDBPersistenceClient::remove' },
        'Failed to delete KV'
      );
    }
  }
}
