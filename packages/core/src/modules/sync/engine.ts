import type { RecordId } from 'surrealdb';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { RemoteDatabaseService } from '../../services/database/index';
import type { CacheModule, CacheRecord, RecordWithId } from '../cache/index';
import type { RecordVersionDiff } from '../../types';
import type { Logger } from '../../services/logger/index';
import { SyncEventTypes, createSyncEventSystem } from './events/index';
import { encodeRecordId } from '../../utils/index';
import { cleanRecord } from '../../utils/parser';

/** Ids per remote `SELECT * FROM $ids` round trip. */
const FETCH_CHUNK = 500;

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

    // Fetch added/updated records from remote. Skip ids whose body the local
    // store already holds at this version (the circuit prime seeded the memo
    // from `_00_rv`): after a reload every id in the server's list_ref used to
    // classify as `added` against an empty circuit, and this fetched the
    // whole working set again.
    const toFetch = [...added, ...updated].filter((item) => {
      const localVersion = this.cache.lookup(encodeRecordId(item.id));
      return !(localVersion && item.version <= localVersion);
    });
    if (toFetch.length === 0) {
      return { remoteFetchMs: 0, stillRemoteIds };
    }

    // Build a version map from the diff (versions come from _00_list_ref)
    const versionMap = new Map<string, number>();
    for (const item of toFetch) {
      versionMap.set(encodeRecordId(item.id), item.version);
    }
    const addedIds = new Set(added.map((item) => encodeRecordId(item.id)));

    // Fetch records from remote — avoid SELECT *, <subquery> FROM $param
    // pattern which drops the * fields in SurrealDB v3 (known bug).
    // Versions are already known from the diff's list_ref data.
    //
    // Chunked so a genuinely cold load lands progressively: one response
    // holding thousands of bodies, then one transaction MERGEing all of them,
    // was a single main-thread stall and a memory spike on top of it.
    let remoteFetchMs = 0;
    const remoteResults: RecordWithId[] = [];
    for (let i = 0; i < toFetch.length; i += FETCH_CHUNK) {
      const idsToFetch = toFetch.slice(i, i + FETCH_CHUNK).map((x) => x.id);
      const remoteFetchStart = performance.now();
      const [chunkResults] = await this.remote.query<[RecordWithId[]]>(
        'SELECT * FROM $idsToFetch',
        { idsToFetch }
      );
      remoteFetchMs += performance.now() - remoteFetchStart;

      // Prepare batch for cache (which handles both DB and DBSP)
      const cacheBatch: CacheRecord[] = [];
      for (const record of chunkResults ?? []) {
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
        remoteResults.push(record);
        const fullId = encodeRecordId(record.id);
        const table = record.id.table.toString();
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
          op: addedIds.has(fullId) ? 'CREATE' : 'UPDATE',
          record: cleanedRecord as RecordWithId,
          version,
        });
      }

      // Use CacheModule to handle both local DB and DBSP ingestion
      if (cacheBatch.length > 0) {
        await this.cache.saveBatch(cacheBatch);
      }
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
    // records directly from the id array (the records ARE the FROM target).
    //
    // The exact query form matters on SurrealDB v3.x:
    //   - `WHERE id IN $ids` is broken: record-id `IN` matches nothing, so every
    //     removed id looked gone and live local records got deleted (a fresh
    //     collection vanished mid-session). Do NOT use `IN`.
    //   - `SELECT id FROM $ids` — a FIELD projection over a record-id ARRAY —
    //     errors "Specify a database to use" on the deployed engine. The catch
    //     below then swallowed it and skipped EVERY deletion, so nothing could be
    //     deleted anywhere (games, comments, …). Do NOT project a field over the
    //     array. (`SELECT * FROM $ids` works but pulls full records — wasteful.)
    //   - `SELECT VALUE id FROM $ids` works: a flat array of ids with a NONE entry
    //     for each id that no longer exists. We filter the NONE entries out; the
    //     survivors are the ids still present upstream.
    let existingRemoteIds: Set<string>;
    try {
      const [existing] = await this.remote.query<[(RecordId | null | undefined)[]]>(
        'SELECT VALUE id FROM $ids',
        { ids: removed }
      );
      existingRemoteIds = new Set(
        (existing ?? [])
          .filter((id): id is RecordId => id != null)
          .map((id) => encodeRecordId(id))
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
