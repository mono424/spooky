import type { RecordId } from 'surrealdb';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { RemoteDatabaseService } from '../../services/database/index';
import type { CacheModule, CacheRecord, RecordWithId } from '../cache/index';
import type { RecordVersionDiff } from '../../types';
import type { Logger } from '../../services/logger/index';
import { SyncEventTypes, createSyncEventSystem } from './events/index';
import { encodeRecordId } from '../../utils/index';
import { cleanRecord } from '../../utils/parser';

/**
 * SyncEngine handles the core sync operations: fetching remote records,
 * caching them locally, and ingesting into DBSP.
 *
 * This is extracted from Sp00kySync to separate "how to sync" from "when to sync".
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
    this.logger = logger.child({ service: 'Sp00kySync:SyncEngine' });
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
        Category: 'sp00ky-client::SyncEngine::syncRecords',
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

    // Build a version map from the diff (versions come from _00_list_ref)
    const versionMap = new Map<string, number>();
    for (const item of toFetch) {
      versionMap.set(encodeRecordId(item.id), item.version);
    }

    // Fetch records from remote — avoid SELECT *, <subquery> FROM $param
    // pattern which drops the * fields in SurrealDB v3 (known bug).
    // Versions are already known from the diff's list_ref data.
    const [remoteResults] = await this.remote.query<[RecordWithId[]]>(
      'SELECT * FROM $idsToFetch',
      { idsToFetch }
    );

    // Prepare batch for cache (which handles both DB and DBSP)
    const cacheBatch: CacheRecord[] = [];

    for (const record of remoteResults) {
      if (!record?.id) {
        this.logger.warn(
          {
            record,
            idsToFetch,
            Category: 'sp00ky-client::SyncEngine::syncRecords',
          },
          'Remote record has no id (possibly deleted). Skipping record'
        );
        continue;
      }
      const fullId = encodeRecordId(record.id);
      const table = record.id.table.toString();
      const isAdded = added.some((item) => encodeRecordId(item.id) === fullId);
      const version = versionMap.get(fullId) ?? 0;

      const localVersion = this.cache.lookup(fullId);
      if (localVersion && version <= localVersion) {
        this.logger.info(
          {
            recordId: fullId,
            version,
            localVersion,
            Category: 'sp00ky-client::SyncEngine::syncRecords',
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
        version,
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
   *
   * "Removed" here is a derived signal: the SSP's `_00_list_ref` array no
   * longer references a record that exists locally. That can mean the row
   * was genuinely deleted upstream — but it can also be a benign race
   * (e.g. a record we just created hasn't propagated into the SSP's
   * incantation list yet). Before deleting locally we verify against
   * upstream SurrealDB: if the row still exists there, skip the delete.
   *
   * On verification failure we skip deletion too. Losing a stale local
   * row to a later sync round is recoverable; deleting a fresh row that
   * upstream still has is not.
   */
  private async handleRemovedRecords(removed: RecordId[]): Promise<void> {
    this.logger.debug(
      {
        removed: removed.map((r) => r.toString()),
        Category: 'sp00ky-client::SyncEngine::handleRemovedRecords',
      },
      'Checking removed records'
    );

    // Group by table so we can issue `SELECT id FROM type::table($t)
    // WHERE id IN $ids` per table. The earlier shape `SELECT id FROM
    // $ids` returns Internal/0 in SurrealDB 3.0 when `$ids` is bound as
    // an array of RecordIds; this form works because the array shows up
    // only in the WHERE clause, not as the FROM target.
    const byTable = new Map<string, RecordId[]>();
    for (const r of removed) {
      const list = byTable.get(r.table.name) ?? [];
      list.push(r);
      byTable.set(r.table.name, list);
    }

    let existingRemoteIds: Set<string>;
    try {
      existingRemoteIds = new Set();
      for (const [table, ids] of byTable) {
        const [existing] = await this.remote.query<[{ id: RecordId }[]]>(
          'SELECT id FROM type::table($table) WHERE id IN $ids',
          { table, ids }
        );
        for (const row of existing) {
          existingRemoteIds.add(encodeRecordId(row.id));
        }
      }
    } catch (err) {
      // Verification failed. Skip deletion entirely — the next sync
      // round re-derives the diff and we get another shot. The
      // alternative (delete on uncertainty) destroys freshly-created
      // rows when the SSP hasn't yet refreshed `_00_list_ref`.
      this.logger.warn(
        {
          err,
          removed: removed.map((r) => r.toString()),
          Category: 'sp00ky-client::SyncEngine::handleRemovedRecords',
        },
        'Remote existence check failed, skipping deletion to avoid clobbering fresh data'
      );
      return;
    }

    for (const recordId of removed) {
      const recordIdStr = encodeRecordId(recordId);
      if (!existingRemoteIds.has(recordIdStr)) {
        this.logger.debug(
          {
            recordId: recordIdStr,
            Category: 'sp00ky-client::SyncEngine::handleRemovedRecords',
          },
          'Deleting confirmed removed record'
        );

        // Use CacheModule to handle both local DB and DBSP deletion
        await this.cache.delete(recordId.table.name, recordIdStr);
      }
    }
  }
}
