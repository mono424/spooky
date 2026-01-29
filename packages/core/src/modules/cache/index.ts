import { LocalDatabaseService } from '../../services/database/index.js';
import {
  StreamProcessorService,
  StreamUpdate,
  StreamUpdateReceiver,
} from '../../services/stream-processor/index.js';
import { Logger } from '../../services/logger/index.js';
import { parseRecordIdString, encodeRecordId, surql } from '../../utils/index.js';
import { CacheRecord, QueryConfig } from './types.js';
import { RecordVersionArray } from '../../types.js';

export * from './types.js';

/**
 * CacheModule - Centralized storage and DBSP ingestion
 *
 * Single responsibility: Handle all local storage operations and DBSP ingestion.
 * This module acts as the bridge between data operations and persistence.
 */
export class CacheModule implements StreamUpdateReceiver {
  private logger: Logger;
  private streamUpdateCallback: (update: StreamUpdate) => void;

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
      { queryHash: update.queryHash, arrayLength: update.localArray?.length },
      'Stream update received'
    );
    this.streamUpdateCallback(update);
  }

  /**
   * Save a single record to local DB and ingest into DBSP
   * Used by mutations (create/update)
   */
  async save(cacheRecord: CacheRecord, isOptimistic: boolean = true): Promise<void> {
    const { table, record, op } = cacheRecord;
    this.logger.debug({ table, op, isOptimistic }, 'Saving record');

    try {
      const { id, ...content } = record;
      await this.local.getClient().upsert(id).content(content);

      // 2. Ingest into DBSP
      this.streamProcessor.ingest(table, op, encodeRecordId(id), record, isOptimistic);

      this.logger.debug({ table, id, op }, 'Record saved successfully');
    } catch (err) {
      this.logger.error({ err, table, record }, 'Failed to save record');
      throw err;
    }
  }

  /**
   * Save multiple records in a batch
   * More efficient than calling save() multiple times
   * Used by sync operations
   */
  async saveBatch(records: CacheRecord[], isOptimistic: boolean = false): Promise<void> {
    if (records.length === 0) return;

    this.logger.debug({ count: records.length, isOptimistic }, 'Saving record batch');

    try {
      const populatedRecords = records.map((record) => {
        return {
          ...record,
          record: {
            ...record.record,
            _spooky_version: record.version,
          },
        };
      });

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

      console.log('abc123', records);

      // 2. Batch ingest into DBSP
      this.streamProcessor.ingestBatch(
        records.map((record) => ({
          ...record,
          record: { ...record.record, id: encodeRecordId(record.record.id) },
        })),
        isOptimistic
      );

      console.log('abc1234');

      this.logger.debug({ count: records.length }, 'Batch saved successfully');
    } catch (err) {
      console.log('xxx222', err);
      this.logger.error({ err, count: records.length }, 'Failed to save batch');
      throw err;
    }
  }

  /**
   * Delete a record from local DB and ingest deletion into DBSP
   */
  async delete(table: string, id: string, isOptimistic: boolean = true): Promise<void> {
    this.logger.debug({ table, id, isOptimistic }, 'Deleting record');

    try {
      // 1. Delete from local database
      await this.local.query('DELETE $id', { id });

      // 2. Ingest deletion into DBSP
      this.streamProcessor.ingest(table, 'DELETE', id, {}, isOptimistic);

      this.logger.debug({ table, id }, 'Record deleted successfully');
    } catch (err) {
      this.logger.error({ err, table, id }, 'Failed to delete record');
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
        { queryHash: config.queryHash, arrayLength: update.localArray?.length },
        'Query registered successfully'
      );

      return { localArray: update.localArray };
    } catch (err) {
      this.logger.error({ err, queryHash: config.queryHash }, 'Failed to register query');
      throw err;
    }
  }

  /**
   * Unregister a query from DBSP
   */
  unregisterQuery(queryHash: string): void {
    this.logger.debug({ queryHash }, 'Unregistering query');
    try {
      this.streamProcessor.unregisterQueryPlan(queryHash);
      this.logger.debug({ queryHash }, 'Query unregistered successfully');
    } catch (err) {
      this.logger.error({ err, queryHash }, 'Failed to unregister query');
    }
  }

  /**
   * Set the version of a record in a specific query view
   * Used during remote sync
   */
  setRecordVersion(queryHash: string, recordId: string, version: number): void {
    this.streamProcessor.setRecordVersion(queryHash, recordId, version);
  }
}
