import type { LocalStore } from '../../services/database/index';
import { StaleEpochError } from '../../services/database/index';
import type {
  StreamProcessorService,
  StreamUpdate,
  StreamUpdateReceiver,
} from '../../services/stream-processor/index';
import type { Logger } from '../../services/logger/index';
import { parseRecordIdString, encodeRecordId, surql } from '../../utils/index';
import type { CacheRecord, QueryConfig } from './types';
import type { RecordVersionArray } from '../../types';

export * from './types';

/**
 * CacheModule - Centralized storage and DBSP ingestion
 *
 * Single responsibility: Handle all local storage operations and DBSP ingestion.
 * This module acts as the bridge between data operations and persistence.
 */
/** One ingested change, in exactly the shape `ingestMany` consumes. Shared
 *  with the tabs protocol so a leader can relay its ingests to followers. */
export interface CacheIngestTuple {
  table: string;
  op: 'CREATE' | 'UPDATE' | 'DELETE';
  id: string;
  record: Record<string, unknown>;
}

export class CacheModule implements StreamUpdateReceiver {
  private logger: Logger;
  private streamUpdateCallback: (update: StreamUpdate) => void;
  private versionLookups: Record<string, number> = {};
  /** Shared-tabs leader: fan every committed ingest out to follower circuits.
   *  Fired AFTER the local tx (the rows are already in the shared store, so a
   *  follower only needs the circuit feed). Never set on followers. */
  private ingestRelay: ((tuples: CacheIngestTuple[]) => void) | null = null;

  constructor(
    private local: LocalStore,
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
        Category: 'sp00ky-client::CacheModule::onStreamUpdate',
      },
      'Stream update received'
    );
    this.streamUpdateCallback(update);
  }

  setIngestRelay(cb: ((tuples: CacheIngestTuple[]) => void) | null): void {
    this.ingestRelay = cb;
  }

  /**
   * Shared-tabs follower: feed relayed tuples into THIS tab's circuit only.
   * The rows are already in the shared store (the leader wrote them), so no
   * local write happens here; the normal chain then runs: SSP -> stream update
   * -> DataModule debounce -> materializeRecords (re-reads via the port
   * transport) -> this tab's subscriptions fire with this tab's hashes.
   */
  applyRelayedIngest(tuples: CacheIngestTuple[]): void {
    for (const t of tuples) {
      const rv = (t.record as { _00_rv?: number } | undefined)?._00_rv;
      if (t.op === 'DELETE') delete this.versionLookups[t.id];
      else if (typeof rv === 'number') this.versionLookups[t.id] = rv;
    }
    this.streamProcessor.ingestMany(tuples as Parameters<StreamProcessorService['ingestMany']>[0]);
  }

  public lookup(recordId: string): number {
    return this.versionLookups[recordId] ?? 0;
  }

  /**
   * Seed the version memo from rows the circuit was primed with out of the
   * local store, so the first post-reload sync diff does not re-download
   * bodies the browser already has. Only rows the prime actually put into the
   * circuit belong here: a memo entry with no circuit row would make the diff
   * flag the id forever while nothing ever fetches it.
   */
  public primeVersions(entries: [string, number][]): void {
    for (const [id, rv] of entries) {
      if (rv > (this.versionLookups[id] ?? 0)) this.versionLookups[id] = rv;
    }
  }

  /** Drop the version cache on a bucket switch — a stale version would make
   *  the sync diff skip fetching a body the new bucket legitimately needs. */
  public clearVersionLookups(): void {
    this.versionLookups = {};
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

    // Fence against bucket switches: this batch's records were derived from
    // reads against the CURRENT store/user. If a switch lands while we await
    // the (gated) local write, the write throws StaleEpochError and the whole
    // batch — including the SSP ingest — is dropped: the new bucket re-syncs
    // its own data from the server.
    const epoch = this.local.epoch;

    this.logger.debug(
      {
        count: records.length,
        Category: 'sp00ky-client::CacheModule::saveBatch',
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
            _00_rv: record.version,
          },
        };
      });

      if (!skipDbInsert) {
        const query = surql.seal<void>(
          surql.tx(
            populatedRecords.map((_, i) => {
              // MERGE, not REPLACE: the remote payload omits local-only
              // fields (`_00_crdt`, `_00_cursor`) injected by the CLI's
              // local schema, so REPLACE would wipe the persisted CRDT
              // snapshot on every sync-down round-trip and break offline
              // reload of formatted text.
              return surql.upsertMerge(`id${i}`, `content${i}`);
            })
          )
        );

        // Filled in place, NOT with a spread-per-iteration reduce: spreading the
        // accumulator copies every key written so far on each record, which is
        // O(n^2) in the batch size. On a cold start that batches thousands of
        // rows (a game library, a player-name registry) it was the single
        // biggest main-thread cost of the whole boot - ~36% of samples, seconds
        // of blocking - for a loop that does no real work.
        const params: Record<string, any> = {};
        for (let i = 0; i < populatedRecords.length; i++) {
          const { id, ...content } = populatedRecords[i].record;
          params[`id${i}`] = id;
          params[`content${i}`] = content;
        }

        await this.local.execute(query, params, { epoch });
      }

      // Late fence for the skipDbInsert path (no gated write above to trip on).
      if (this.local.epoch !== epoch) throw new StaleEpochError();

      // 2. Bulk ingest into DBSP (use populatedRecords which has _00_rv set).
      // ingestMany coalesces the per-record stream updates into a single
      // notification per affected query — the UI then updates once, after the
      // whole batch is ingested, instead of row-by-row.
      const versionOf = new Map<string, number>();
      const bulk = populatedRecords.map((record) => {
        const recordId = encodeRecordId(record.record.id);
        versionOf.set(recordId, record.version);
        return { table: record.table, op: record.op, id: recordId, record: record.record };
      });
      // Memo and relay only what the circuit actually took: a chunk that
      // failed mid-batch is neither "known" here nor fanned out to followers.
      const ingested = this.streamProcessor.ingestMany(bulk);
      for (const t of ingested) this.versionLookups[t.id] = versionOf.get(t.id) ?? 0;
      if (ingested.length > 0) this.ingestRelay?.(ingested as CacheIngestTuple[]);

      this.logger.debug(
        { count: records.length, Category: 'sp00ky-client::CacheModule::saveBatch' },
        'Batch saved successfully'
      );
    } catch (err) {
      if (err instanceof StaleEpochError) {
        this.logger.debug(
          { count: records.length, Category: 'sp00ky-client::CacheModule::saveBatch' },
          'Dropped batch from before a bucket switch'
        );
        return;
      }
      this.logger.error(
        { err, count: records.length, Category: 'sp00ky-client::CacheModule::saveBatch' },
        'Failed to save batch'
      );
      throw err;
    }
  }

  /**
   * Delete a record from local DB and ingest deletion into DBSP
   */
  async delete(table: string, id: string, skipDbDelete: boolean = false, recordData: Record<string, any> = {}): Promise<void> {
    this.logger.debug(
      { table, id, Category: 'sp00ky-client::CacheModule::delete' },
      'Deleting record'
    );

    const epoch = this.local.epoch;
    try {
      // 1. Delete from local database
      if (!skipDbDelete) {
        await this.local.query('DELETE $id', { id: parseRecordIdString(id) }, { epoch });
      }
      if (this.local.epoch !== epoch) throw new StaleEpochError();

      // 2. Ingest deletion into DBSP (pass record data so predicates can be matched)
      delete this.versionLookups[id];
      this.streamProcessor.ingestMany([{ table, op: 'DELETE', id, record: recordData }]);
      this.ingestRelay?.([{ table, op: 'DELETE', id, record: recordData }]);

      this.logger.debug(
        { table, id, Category: 'sp00ky-client::CacheModule::delete' },
        'Record deleted successfully'
      );
    } catch (err) {
      if (err instanceof StaleEpochError) {
        this.logger.debug(
          { table, id, Category: 'sp00ky-client::CacheModule::delete' },
          'Dropped delete from before a bucket switch'
        );
        return;
      }
      this.logger.error(
        { err, table, id, Category: 'sp00ky-client::CacheModule::delete' },
        'Failed to delete record'
      );
      throw err;
    }
  }

  /**
   * Register a query with DBSP to create a materialized view
   * Returns the initial result array
   */
  registerQuery(config: QueryConfig): {
    localArray: RecordVersionArray;
    registrationTimings?: { parseMs: number; planMs: number; snapshotMs: number };
  } {
    this.logger.debug(
      {
        queryHash: config.queryHash,
        surql: config.surql,
        Category: 'sp00ky-client::CacheModule::registerQuery',
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
          Category: 'sp00ky-client::CacheModule::registerQuery',
        },
        'Query registered successfully'
      );

      return { localArray: update.localArray, registrationTimings: update.registration };
    } catch (err) {
      this.logger.error(
        { err, queryHash: config.queryHash, Category: 'sp00ky-client::CacheModule::registerQuery' },
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
      { queryHash, Category: 'sp00ky-client::CacheModule::unregisterQuery' },
      'Unregistering query'
    );
    try {
      this.streamProcessor.unregisterQueryPlan(queryHash);
      this.logger.debug(
        { queryHash, Category: 'sp00ky-client::CacheModule::unregisterQuery' },
        'Query unregistered successfully'
      );
    } catch (err) {
      this.logger.error(
        { err, queryHash, Category: 'sp00ky-client::CacheModule::unregisterQuery' },
        'Failed to unregister query'
      );
    }
  }
}
