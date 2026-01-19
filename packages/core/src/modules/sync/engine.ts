import { RecordId } from 'surrealdb';
import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { StreamProcessorService } from '../../services/stream-processor/index.js';
import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { ArraySyncer } from './utils.js';
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
  public events = createSyncEventSystem();

  constructor(
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    private streamProcessor: StreamProcessorService,
    private logger: Logger
  ) {}

  /**
   * Sync missing/updated/removed records between local and remote.
   * Main entry point for sync operations.
   * Uses batch processing to minimize events emitted.
   */
  async syncRecords(diff: RecordVersionDiff): Promise<RecordVersionDiff> {
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
      return diff;
    }

    const [remoteResults] = await this.remote.query<[Record<string, any>[]]>(
      "SELECT *, (SELECT version FROM ONLY _spooky_version WHERE record_id = <record>$parent.id)['version'] as _spookyv FROM $ids",
      {
        ids: idsToFetch,
      }
    );

    // BATCH PROCESSING: Cache all records locally first
    const cachePromises = remoteResults.map(async (record) => {
      await this.local.getClient().upsert(record.id).content(record);
    });
    await Promise.all(cachePromises);

    // Prepare batch for ingestion
    const ingestBatch: Array<{
      table: string;
      op: string;
      id: string;
      record: any;
      version: number;
    }> = [];

    for (const { _spookyv, ...record } of remoteResults) {
      const fullId = record.id.toString();
      const table = record.id.table.toString();
      const isAdded = added.some((item) => item.id.toString() === fullId);

      const anticipatedVersion = toFetch.find((item) => item.id.toString() === fullId)?.version;
      if (anticipatedVersion && _spookyv < anticipatedVersion) {
        this.logger.warn(
          { recordId: fullId, version: _spookyv, anticipatedVersion },
          'Received outdated record version. Skipping record __TEST__'
        );
        continue;
      }

      ingestBatch.push({
        table,
        op: isAdded ? 'CREATE' : 'UPDATE',
        id: fullId,
        record,
        version: _spookyv,
      });
    }
    console.log('__TEST__', diff, ingestBatch);
    // Single batch ingest call (isOptimistic=false for remote sync)
    if (ingestBatch.length > 0) {
      this.streamProcessor.ingestBatch(ingestBatch, true);
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: remoteResults,
    });

    return diff;
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

        // 1. Delete from local DB
        await this.local.query('DELETE $id', { id: recordIdStr });

        // 2. Ingest deletion into DBSP
        this.streamProcessor.ingest(recordId.table.name, 'DELETE', recordIdStr, {}, false);
      }
    }
  }
}
