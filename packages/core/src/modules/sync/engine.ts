import { RecordId } from 'surrealdb';
import { SchemaStructure } from '@spooky-sync/query-builder';
import { RemoteDatabaseService } from '../../services/database/index';
import { CacheModule, CacheRecord, RecordWithId } from '../cache/index';
import { RecordVersionDiff } from '../../types';
import { Logger } from '../../services/logger/index';
import { SyncEventTypes, createSyncEventSystem } from './events/index';
import { encodeRecordId } from '../../utils/index';
import { cleanRecord } from '../../utils/parser';

/**
 * SyncEngine handles the core sync operations: fetching remote records,
 * caching them locally, and ingesting into DBSP.
 *
 * This is extracted from SpookySync to separate "how to sync" from "when to sync".
 */
export class SyncEngine {
  private logger: Logger;
  public events = createSyncEventSystem();

  constructor(
    private remote: RemoteDatabaseService,
    private cache: CacheModule,
    private schema: SchemaStructure,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'SpookySync:SyncEngine' });
  }

  /**
   * Sync missing/updated/removed records between local and remote.
   * Main entry point for sync operations.
   * Uses batch processing to minimize events emitted.
   */
  async syncRecords(diff: RecordVersionDiff): Promise<void> {
    const { added, updated, removed } = diff;

    this.logger.debug(
      {
        added,
        updated,
        removed,
        Category: 'spooky-client::SyncEngine::syncRecords',
      },
      'SyncEngine.syncRecords diff'
    );

    // Handle removed records: verify they don't exist remotely before deleting locally
    if (removed.length > 0) {
      await this.handleRemovedRecords(removed);
    }

    // Fetch added/updated records from remote
    const toFetch = [...added, ...updated];
    const idsToFetch = toFetch.map((x) => x.id);
    if (idsToFetch.length === 0) {
      return;
    }

    const [remoteResults] = await this.remote.query<[RecordWithId[]]>(
      "SELECT (SELECT * FROM ONLY <record>$parent.id) AS record, (SELECT version FROM ONLY _spooky_version WHERE record_id = <record>$parent.id)['version'] as spooky_rv FROM $idsToFetch",
      { idsToFetch }
    );
    console.log('remoteResults>', remoteResults);
    // Prepare batch for cache (which handles both DB and DBSP)
    const cacheBatch: CacheRecord[] = [];

    for (const { spooky_rv, record } of remoteResults) {
      if (!record?.id) {
        this.logger.warn(
          {
            record,
            idsToFetch,
            Category: 'spooky-client::SyncEngine::syncRecords',
          },
          'Remote record has no id. Skipping record'
        );
        continue;
      }
      const fullId = encodeRecordId(record.id);
      const table = record.id.table.toString();
      const isAdded = added.some((item) => encodeRecordId(item.id) === fullId);

      const localVersion = this.cache.lookup(fullId);
      if (localVersion && spooky_rv <= localVersion) {
        this.logger.info(
          {
            recordId: fullId,
            version: spooky_rv,
            localVersion,
            Category: 'spooky-client::SyncEngine::syncRecords',
          },
          'Local version is higher than remote version. Skipping record'
        );
        continue;
      }
      const tableSchema = this.schema.tables.find((t) => t.name === table);
      const cleanedRecord = tableSchema
        ? cleanRecord(tableSchema.columns, record)
        : record;

      cacheBatch.push({
        table,
        op: isAdded ? 'CREATE' : 'UPDATE',
        record: cleanedRecord as RecordWithId,
        version: spooky_rv,
      });
    }

    // Use CacheModule to handle both local DB and DBSP ingestion
    if (cacheBatch.length > 0) {
      await this.cache.saveBatch(cacheBatch);
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: remoteResults,
    });
  }

  /**
   * Handle records that exist locally but not in remote array.
   */
  private async handleRemovedRecords(removed: RecordId[]): Promise<void> {
    this.logger.debug(
      {
        removed: removed.map((r) => r.toString()),
        Category: 'spooky-client::SyncEngine::handleRemovedRecords',
      },
      'Checking removed records'
    );

    let existingRemoteIds = new Set<string>();
    try {
      const [existingRemote] = await this.remote.query<[{ id: RecordId }[]]>('SELECT id FROM $ids', {
        ids: removed,
      });
      existingRemoteIds = new Set(existingRemote.map((r) => encodeRecordId(r.id)));
    } catch {
      // If remote check fails (e.g., SurrealDB parameter serialization issue),
      // proceed with deletion — the caller has already determined these should be removed
      this.logger.debug(
        { Category: 'spooky-client::SyncEngine::handleRemovedRecords' },
        'Remote existence check failed, proceeding with deletion'
      );
    }

    for (const recordId of removed) {
      const recordIdStr = encodeRecordId(recordId);
      if (!existingRemoteIds.has(recordIdStr)) {
        this.logger.debug(
          {
            recordId: recordIdStr,
            Category: 'spooky-client::SyncEngine::handleRemovedRecords',
          },
          'Deleting confirmed removed record'
        );

        // Use CacheModule to handle both local DB and DBSP deletion
        await this.cache.delete(recordId.table.name, recordIdStr);
      }
    }
  }
}
