import { LocalDatabaseService } from '../../services/database/index';
import {
  StreamProcessorService,
  StreamUpdate,
  StreamUpdateReceiver,
} from '../../services/stream-processor/index';
import { Logger } from '../../services/logger/index';
import { parseRecordIdString, encodeRecordId, surql } from '../../utils/index';
import { CacheRecord, QueryConfig } from './types';
import { RecordVersionArray } from '../../types';

export * from './types';

/**
 * CacheModule - Centralized storage and DBSP ingestion
 *
 * Single responsibility: Handle all local storage operations and DBSP ingestion.
 * This module acts as the bridge between data operations and persistence.
 */
export class CacheModule implements StreamUpdateReceiver {
  private logger: Logger;
  private streamUpdateCallback: (update: StreamUpdate) => void;
  private versionLookups: Record<string, number> = {};

  constructor(
    private local: LocalDatabaseService,
    private streamProcessor: StreamProcessorService,
    streamUpdateCallback: (update: StreamUpdate) => void,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'CacheModule' });
    this.streamUpdateCallback = streamUpdateCallback;
    // Register as receiver for DBSP stream updates
    this.streamProcessor.addReceiver(this);
  }

  /**
   * Implements StreamUpdateReceiver interface
   * Called directly by StreamProcessor when views change
   */
  onStreamUpdate(update: StreamUpdate): void {
    this.logger.debug(
      {
        queryHash: update.queryHash,
        arrayLength: update.localArray?.length,
        Category: 'spooky-client::CacheModule::onStreamUpdate',
      },
      'Stream update received'
    );
    this.streamUpdateCallback(update);
  }

  public lookup(recordId: string): number {
    return this.versionLookups[recordId] ?? 0;
  }

  /**
   * Save a single record to local DB and ingest into DBSP
   * Used by mutations (create/update)
   */
  async save(cacheRecord: CacheRecord, skipDbInsert: boolean = false): Promise<void> {
    return this.saveBatch([cacheRecord], skipDbInsert);
  }

  /**
   * Save multiple records in a batch
   * More efficient than calling save() multiple times
   * Used by sync operations
   */
  async saveBatch(records: CacheRecord[], skipDbInsert: boolean = false): Promise<void> {
    if (records.length === 0) return;

    this.logger.debug(
      {
        count: records.length,
        Category: 'spooky-client::CacheModule::saveBatch',
      },
      'Saving record batch'
    );

    try {
      const populatedRecords = records.map((record) => {
        if (!record.version) throw new Error('Record version is required');
        return {
          ...record,
          record: {
            ...record.record,
            spooky_rv: record.version,
          },
        };
      });

      if (!skipDbInsert) {
        const query = surql.seal(
          surql.tx(
            populatedRecords.map((_, i) => {
              return surql.upsert(`id${i}`, `content${i}`);
            })
          )
        );

        const params = populatedRecords.reduce(
          (acc, record, i) => {
            const { id, ...content } = record.record;
            return {
              ...acc,
              [`id${i}`]: id,
              [`content${i}`]: content,
            };
          },
          {} as Record<string, any>
        );

        await this.local.query(query, params);
      }

      // 2. Batch ingest into DBSP
      for (const record of records) {
        const recordId = encodeRecordId(record.record.id);
        this.versionLookups[recordId] = record.version;
        this.streamProcessor.ingest(record.table, record.op, recordId, record.record);
      }

      this.logger.debug(
        { count: records.length, Category: 'spooky-client::CacheModule::saveBatch' },
        'Batch saved successfully'
      );
    } catch (err) {
      this.logger.error(
        { err, count: records.length, Category: 'spooky-client::CacheModule::saveBatch' },
        'Failed to save batch'
      );
      throw err;
    }
  }

  /**
   * Delete a record from local DB and ingest deletion into DBSP
   */
  async delete(table: string, id: string, skipDbDelete: boolean = false): Promise<void> {
    this.logger.debug(
      { table, id, Category: 'spooky-client::CacheModule::delete' },
      'Deleting record'
    );

    try {
      // 1. Delete from local database
      if (!skipDbDelete) {
        await this.local.query('DELETE $id', { id: parseRecordIdString(id) });
      }

      // 2. Ingest deletion into DBSP
      delete this.versionLookups[id];
      await this.streamProcessor.ingest(table, 'DELETE', id, {});

      this.logger.debug(
        { table, id, Category: 'spooky-client::CacheModule::delete' },
        'Record deleted successfully'
      );
    } catch (err) {
      this.logger.error(
        { err, table, id, Category: 'spooky-client::CacheModule::delete' },
        'Failed to delete record'
      );
      throw err;
    }
  }

  /**
   * Register a query with DBSP to create a materialized view
   * Returns the initial result array
   */
  registerQuery(config: QueryConfig): { localArray: RecordVersionArray } {
    this.logger.debug(
      {
        queryHash: config.queryHash,
        surql: config.surql,
        Category: 'spooky-client::CacheModule::registerQuery',
      },
      'Registering query'
    );

    try {
      const update = this.streamProcessor.registerQueryPlan({
        queryHash: config.queryHash,
        surql: config.surql,
        params: config.params,
        ttl: config.ttl,
        lastActiveAt: config.lastActiveAt,
        localArray: [],
        remoteArray: [],
        meta: {
          tableName: '',
        },
      });

      if (!update) {
        throw new Error('Failed to register query with DBSP');
      }

      this.logger.debug(
        {
          queryHash: config.queryHash,
          arrayLength: update.localArray?.length,
          Category: 'spooky-client::CacheModule::registerQuery',
        },
        'Query registered successfully'
      );

      return { localArray: update.localArray };
    } catch (err) {
      this.logger.error(
        { err, queryHash: config.queryHash, Category: 'spooky-client::CacheModule::registerQuery' },
        'Failed to register query'
      );
      throw err;
    }
  }

  /**
   * Unregister a query from DBSP
   */
  unregisterQuery(queryHash: string): void {
    this.logger.debug(
      { queryHash, Category: 'spooky-client::CacheModule::unregisterQuery' },
      'Unregistering query'
    );
    try {
      this.streamProcessor.unregisterQueryPlan(queryHash);
      this.logger.debug(
        { queryHash, Category: 'spooky-client::CacheModule::unregisterQuery' },
        'Query unregistered successfully'
      );
    } catch (err) {
      this.logger.error(
        { err, queryHash, Category: 'spooky-client::CacheModule::unregisterQuery' },
        'Failed to unregister query'
      );
    }
  }
}
