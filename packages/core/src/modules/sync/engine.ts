import { RecordId } from 'surrealdb';
import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { StreamProcessorService } from '../../services/stream-processor/index.js';
import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { ArraySyncer } from './utils.js';
import { Logger } from '../../services/logger/index.js';
import { SyncEventTypes, createSyncEventSystem } from './events/index.js';

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
    private relationshipMap: Map<string, Set<string>>,
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
    const { added, updated, removed } = diff;
    const idsToFetch = [...added, ...updated];

    this.logger.debug({ added, updated, removed, idsToFetch }, 'SyncEngine.syncRecords diff');

    // Build remote version map for quick lookup
    const remoteVersionMap = new Map<string, number>();
    for (const [id, ver] of remoteArray) {
      remoteVersionMap.set(id.toString(), ver);
    }

    // Handle removed records: verify they don't exist remotely before deleting locally
    if (removed.length > 0) {
      await this.handleRemovedRecords(removed, incantationId, arraySyncer);
    }

    // Fetch added/updated records from remote
    if (idsToFetch.length === 0) {
      return { added: [], updated: [], removed };
    }

    const [remoteResults] = await this.remote.query<[Record<string, any>[]]>('SELECT * FROM $ids', {
      ids: idsToFetch,
    });

    // Flatten and prepare results
    const flatResults = this.flattenResults(remoteResults);

    // BATCH PROCESSING: Cache all records locally first
    const cachePromises = flatResults.map(async (record) => {
      await this.local.getClient().upsert(record.id).content(record);
    });
    await Promise.all(cachePromises);

    // Prepare batch for ingestion
    const ingestBatch: Array<{ table: string; op: string; id: string; record: any }> = [];
    const versionUpdates: Array<{ fullId: string; version: number }> = [];

    for (const record of flatResults) {
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
        versionUpdates.push({ fullId, version: remoteVer });
      }
    }

    // Single batch ingest call (isOptimistic=false for remote sync)
    if (ingestBatch.length > 0) {
      this.streamProcessor.ingestBatch(ingestBatch, false);
    }

    // Set versions for all records
    for (const { fullId, version } of versionUpdates) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, version);
    }

    // Update arraySyncer with final state
    const finalVersionArray: RecordVersionArray = versionUpdates.map(({ fullId, version }) => [
      fullId,
      version,
    ]);
    if (finalVersionArray.length > 0) {
      arraySyncer.update(finalVersionArray);
    }

    this.events.emit(SyncEventTypes.RemoteDataIngested, {
      records: flatResults,
    });

    return { added, updated, removed };
  }

  /**
   * Handle records that exist locally but not in remote array.
   */
  private async handleRemovedRecords(
    removed: RecordId[],
    incantationId: RecordId<string>,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    this.logger.debug({ removed: removed.map((r) => r.toString()) }, 'Checking removed records');

    const [existingRemote] = await this.remote.query<[{ id: RecordId }[]]>('SELECT id FROM $ids', {
      ids: removed,
    });
    const existingRemoteIds = new Set(existingRemote.map((r) => r.id.toString()));

    for (const recordId of removed) {
      if (!existingRemoteIds.has(recordId.toString())) {
        this.logger.debug({ recordId: recordId.toString() }, 'Deleting confirmed removed record');
        await this.deleteCacheEntry(recordId, incantationId, arraySyncer);
      }
    }
  }

  /**
   * Create a record in local cache and ingest into DBSP.
   */
  private async createCacheEntry(
    record: Record<string, any>,
    incantationId: RecordId<string>,
    remoteVersion: number | undefined,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = record.id.table.toString();
    const fullId = record.id.toString();

    // 1. Cache in local DB
    await this.local.getClient().upsert(record.id).content(record);

    // 2. Ingest into DBSP
    const result = await this.streamProcessor.ingest(table, 'CREATE', fullId, record, true);

    // 3. Set version if provided
    if (remoteVersion !== undefined) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, remoteVersion);
      arraySyncer.update([[fullId, remoteVersion]] as RecordVersionArray);
    } else {
      for (const update of result) {
        if (update.query_id === incantationId.toString()) {
          arraySyncer.update(update.result_data);
        }
      }
    }
  }

  /**
   * Update a record in local cache and ingest into DBSP.
   */
  private async updateCacheEntry(
    record: Record<string, any>,
    incantationId: RecordId<string>,
    remoteVersion: number | undefined,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = record.id.table.toString();
    const fullId = record.id.toString();

    // 1. Cache in local DB
    await this.local.getClient().upsert(record.id).content(record);

    // 2. Ingest into DBSP (isOptimistic=false to not auto-increment version)
    const result = await this.streamProcessor.ingest(table, 'UPDATE', fullId, record, false);

    // 3. Set version if provided
    if (remoteVersion !== undefined) {
      this.streamProcessor.setRecordVersion(incantationId.toString(), fullId, remoteVersion);
      arraySyncer.update([[fullId, remoteVersion]] as RecordVersionArray);
    } else {
      for (const update of result) {
        if (update.query_id === incantationId.toString()) {
          arraySyncer.update(update.result_data);
        }
      }
    }
  }

  /**
   * Delete a record from local cache and ingest deletion into DBSP.
   */
  private async deleteCacheEntry(
    recordId: RecordId,
    incantationId: RecordId<string>,
    arraySyncer: ArraySyncer
  ): Promise<void> {
    const table = recordId.table.toString();
    const fullId = recordId.toString();

    // 1. Delete from local DB
    await this.local.query('DELETE $id', { id: recordId });

    // 2. Ingest deletion into DBSP
    const result = await this.streamProcessor.ingest(table, 'DELETE', fullId, {}, false);

    for (const update of result) {
      if (update.query_id === incantationId.toString()) {
        arraySyncer.update(update.result_data);
      }
    }
  }

  /**
   * Recursively flattens a list of records, extracting nested objects that look like records (have an 'id')
   * into the top-level list, and replacing them with their ID in the parent.
   *
   * Schema-aware: Only flattens fields that are defined as relationships in the schema.
   */
  flattenResults(
    results: Record<string, any>[],
    visited: Set<string> = new Set(),
    flattened: Record<string, any>[] = []
  ): Record<string, any>[] {
    for (const record of results) {
      if (!record) continue;

      // 1. Identify the Record
      let recordIdStr: string | undefined;
      let tableName: string | undefined;

      if (record.id && record.id instanceof RecordId) {
        recordIdStr = record.id.toString();
        tableName = record.id.table.name;
      }

      // 2. Cycle Detection / Deduplication
      if (recordIdStr) {
        if (visited.has(recordIdStr)) continue;
        visited.add(recordIdStr);
      }

      // 3. Create a shallow copy to modify fields without mutating original
      const processedRecord: Record<string, any> = { ...record };

      // 4. Handle Relationships recursively
      if (tableName && this.relationshipMap.has(tableName)) {
        const validFields = this.relationshipMap.get(tableName)!;

        for (const key of validFields) {
          if (!(key in processedRecord)) continue;

          const value = processedRecord[key];

          if (value && typeof value === 'object') {
            if (Array.isArray(value)) {
              processedRecord[key] = this.flattenResults(value, visited, flattened);
            } else if (value.id && value.id instanceof RecordId) {
              this.flattenResults([value], visited, flattened);
              processedRecord[key] = value.id;
            }
          }
        }
      }
      flattened.push(processedRecord);
    }

    return flattened;
  }
}
