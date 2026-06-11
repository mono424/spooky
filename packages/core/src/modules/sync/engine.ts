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
  async syncRecords(
    diff: RecordVersionDiff
  ): Promise<{ remoteFetchMs: number; stillRemoteIds: string[] }> {
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

    // Handle removed records: verify they don't exist remotely before deleting
    // locally. Returns ids that LEFT the view's list_ref but still exist upstream
    // (so they weren't deleted) — the caller converges localArray to drop them.
    let stillRemoteIds: string[] = [];
    if (removed.length > 0) {
      stillRemoteIds = await this.handleRemovedRecords(removed);
    }

    // Fetch added/updated records from remote
    const toFetch = [...added, ...updated];
    const idsToFetch = toFetch.map((x) => x.id);
    if (idsToFetch.length === 0) {
      return { remoteFetchMs: 0, stillRemoteIds };
    }

    // Build a version map from the diff (versions come from _00_list_ref)
    const versionMap = new Map<string, number>();
    for (const item of toFetch) {
      versionMap.set(encodeRecordId(item.id), item.version);
    }

    // Fetch records from remote — avoid SELECT *, <subquery> FROM $param
    // pattern which drops the * fields in SurrealDB v3 (known bug).
    // Versions are already known from the diff's list_ref data.
    const remoteFetchStart = performance.now();
    const [remoteResults] = await this.remote.query<[RecordWithId[]]>(
      'SELECT * FROM $idsToFetch',
      { idsToFetch }
    );
    const remoteFetchMs = performance.now() - remoteFetchStart;

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

    return { remoteFetchMs, stillRemoteIds };
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
  private async handleRemovedRecords(removed: RecordId[]): Promise<string[]> {
    this.logger.debug(
      {
        removed: removed.map((r) => r.toString()),
        Category: 'sp00ky-client::SyncEngine::handleRemovedRecords',
      },
      'Checking removed records'
    );

    // Confirm which of the "removed" ids still exist remotely by selecting the
    // records directly: `SELECT id FROM $ids` (the records to fetch ARE the
    // FROM target).
    //
    // We must NOT use `WHERE id IN $ids` here: on SurrealDB v3.1.x, record-id
    // matching with `IN` is broken — `SELECT id FROM <table> WHERE id IN
    // [<recordid>]` returns NO rows even for an existing record (plain-field
    // `IN` works; record-id `IN` does not). That made this check report EVERY
    // removed id as gone and DELETE live local records (e.g. a freshly-created
    // collection vanished mid-session). `SELECT id FROM $ids` matches correctly.
    // (The old comment claimed `FROM $ids` returned Internal/0 on v3.0; it works
    // on v3.1, and even if it ever errored the catch below skips deletion —
    // strictly safer than the silent empty-result the `IN` form produced.)
    let existingRemoteIds: Set<string>;
    try {
      const [existing] = await this.remote.query<[{ id: RecordId }[]]>(
        'SELECT id FROM $ids',
        { ids: removed }
      );
      existingRemoteIds = new Set(
        (existing ?? []).map((row) => encodeRecordId(row.id))
      );
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
      return [];
    }

    // Ids that left the view's list_ref but STILL exist upstream — not deletions,
    // just a view-membership change (e.g. a record whose field changed so it no
    // longer matches the query). The caller drops these from `localArray` so the
    // poll's diff stops re-flagging them every tick (the `job:` churn).
    const stillRemoteIds: string[] = [];
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
      } else {
        stillRemoteIds.push(recordIdStr);
      }
    }
    return stillRemoteIds;
  }
}
