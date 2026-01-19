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
  async syncRecords(
    arraySyncer: ArraySyncer,
    incantationId: RecordId<string>,
    remoteArray: RecordVersionArray
  ): Promise<RecordVersionDiff> {
    const diff = arraySyncer.nextSet();
    if (!diff) {
      return { added: [], updated: [], removed: [] };
    }

    const { added, updated, removed } = diff;

    this.logger.debug({ added, updated, removed }, 'SyncEngine.syncRecords diff');

    // Build remote version map for quick lookup
    const remoteVersionMap = new Map<string, number>();
    for (const [id, ver] of remoteArray) {
      remoteVersionMap.set(id.toString(), ver);
    }

    // Handle removed records: verify they don't exist remotely before deleting locally
    if (removed.length > 0) {
      await this.handleRemovedRecords(removed, arraySyncer);
    }

    // Fetch added/updated records from remote
    const idsToFetch = [...added, ...updated];
    if (idsToFetch.length === 0) {
      return diff;
    }

    const [remoteResults] = await this.remote.query<[Record<string, any>[]]>('SELECT * FROM $ids', {
      ids: idsToFetch,
    });

    // BATCH PROCESSING: Cache all records locally first
    const cachePromises = remoteResults.map(async (record) => {
      await this.local.getClient().upsert(record.id).content(record);
    });
    await Promise.all(cachePromises);

    // Prepare batch for ingestion
    const ingestBatch: Array<{ table: string; op: string; id: string; record: any }> = [];
    const versionUpdates: Array<{ fullId: string; version: number; isAdded: boolean }> = [];

    for (const record of remoteResults) {
      const fullId = record.id.toString();
      const table = record.id.table.toString();
      const isAdded = added.some((id) => id.toString() === fullId);
      const remoteVer = remoteVersionMap.get(fullId);

      ingestBatch.push({
        table,
        op: isAdded ? 'CREATE' : 'UPDATE',
        id: fullId,
        record,
      });

      if (remoteVer !== undefined) {
        versionUpdates.push({ fullId, version: remoteVer, isAdded });
      }
    }

    // Single batch ingest call (isOptimistic=false for remote sync)
    if (ingestBatch.length > 0) {
      this.streamProcessor.ingestBatch(ingestBatch, false);
    }

    // Set versions for all records
    for (const { fullId, version, isAdded } of versionUpdates) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, version);
      if (isAdded) arraySyncer.insert(fullId, version);
      else arraySyncer.update(fullId, version);
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: remoteResults,
    });

    return diff;
  }

  /**
   * Handle records that exist locally but not in remote array.
   */
  private async handleRemovedRecords(removed: RecordId[], arraySyncer: ArraySyncer): Promise<void> {
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
        const result = await this.streamProcessor.ingest(
          recordId.table.name,
          'DELETE',
          recordIdStr,
          {},
          false
        );

        // 3. Delete from arraySyncer
        arraySyncer.delete(recordIdStr);
      }
    }
  }
}
