import { RecordId } from 'surrealdb';
import { RemoteDatabaseService } from '../../services/database/index.js';
import { CacheModule, RecordWithId } from '../cache/index.js';
import { RecordVersionDiff } from '../../types.js';
import { Logger } from '../../services/logger/index.js';
import { SyncEventTypes, createSyncEventSystem } from './events/index.js';
import { encodeRecordId } from '../../utils/index.js';

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

    this.logger.debug({ added, updated, removed }, 'SyncEngine.syncRecords diff');

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
      "SELECT *, (SELECT version FROM ONLY _spooky_version WHERE record_id = <record>$parent.id)['version'] as _spookyv FROM $ids",
      {
        ids: idsToFetch,
      }
    );

    // Prepare batch for cache (which handles both DB and DBSP)
    const cacheBatch = [];

    for (const { _spookyv, ...record } of remoteResults) {
      const fullId = encodeRecordId(record.id);
      const table = record.id.table.toString();
      const isAdded = added.some((item) => encodeRecordId(item.id) === fullId);

      const anticipatedVersion = toFetch.find(
        (item) => encodeRecordId(item.id) === fullId
      )?.version;
      if (anticipatedVersion && _spookyv < anticipatedVersion) {
        this.logger.warn(
          { recordId: fullId, version: _spookyv, anticipatedVersion },
          'Received outdated record version. Skipping record'
        );
        continue;
      }

      cacheBatch.push({
        table,
        op: isAdded ? 'CREATE' : 'UPDATE',
        id: fullId,
        record,
        version: _spookyv,
      });
    }

    // Use CacheModule to handle both local DB and DBSP ingestion
    if (cacheBatch.length > 0) {
      await this.cache.saveBatch(cacheBatch, false); // isOptimistic=false for remote sync
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: remoteResults,
    });
  }

  /**
   * Handle records that exist locally but not in remote array.
   */
  private async handleRemovedRecords(removed: RecordId[]): Promise<void> {
    this.logger.debug({ removed: removed.map((r) => r.toString()) }, 'Checking removed records');

    const [existingRemote] = await this.remote.query<[{ id: string }[]]>('SELECT id FROM $ids', {
      ids: removed,
    });
    const existingRemoteIds = new Set(existingRemote.map((r) => r.id));

    for (const recordId of removed) {
      const recordIdStr = encodeRecordId(recordId);
      if (!existingRemoteIds.has(recordIdStr)) {
        this.logger.debug({ recordId: recordIdStr }, 'Deleting confirmed removed record');

        // Use CacheModule to handle both local DB and DBSP deletion
        await this.cache.delete(recordId.table.name, recordIdStr, false);
      }
    }
  }
}
