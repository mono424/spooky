import { RecordId } from 'surrealdb';
import type { LocalStore } from '../../../services/database/index';
import type { SyncQueueEventSystem } from '../events/index';
import { createSyncQueueEventSystem, SyncQueueEventTypes } from '../events/index';
import {
  parseRecordIdString,
  extractTablePart,
  classifySyncError,
  encodeRecordId,
} from '../../../utils/index';
import type { Logger } from '../../../services/logger/index';
import type { PushEventOptions } from '../../../events/index';

export type CreateEvent = {
  type: 'create';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
  tableName?: string;
  options?: PushEventOptions;
};

export type UpdateEvent = {
  type: 'update';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
  beforeRecord?: Record<string, unknown>;
  options?: PushEventOptions;
};

export type DeleteEvent = {
  type: 'delete';
  mutation_id: RecordId;
  record_id: RecordId;
  options?: PushEventOptions;
};

export type UpEvent = CreateEvent | UpdateEvent | DeleteEvent;

export type RollbackCallback = (event: UpEvent, error: Error) => Promise<void>;

/**
 * A pending mutation that can never be sent, so it was discarded instead of
 * being retried forever at the head of the queue.
 */
export type DroppedMutation = {
  mutationId: string;
  recordId?: string;
  mutationType?: string;
  reason: string;
};

export type DroppedCallback = (dropped: DroppedMutation) => void;

export class UpQueue {
  private queue: UpEvent[] = [];
  private _events: SyncQueueEventSystem;
  private logger: Logger;
  private debouncedMutations: Map<
    string,
    { timer: any; firstBeforeRecord?: Record<string, unknown> }
  >;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(
    private local: LocalStore,
    logger: Logger,
    private onDropped?: DroppedCallback
  ) {
    this._events = createSyncQueueEventSystem();
    this.logger = logger.child({ service: 'UpQueue' });
    this.debouncedMutations = new Map();
  }

  /**
   * Discard an outbox row that can never be replayed and report it.
   *
   * Silently skipping such a row leaves it in the store to be re-read on every
   * boot; leaving it QUEUED is worse, since `next()` re-queues it on failure and
   * one unsendable row then blocks every later mutation for the whole app. A
   * lost write must also be loud: this is the only signal a caller gets.
   */
  private async discardUnreplayable(row: any, reason: string): Promise<void> {
    const mutationId = typeof row?.id === 'string' ? row.id : encodeRecordId(row?.id);
    this.logger.error(
      {
        mutationId,
        recordId: row?.recordId,
        mutationType: row?.mutationType,
        reason,
        Category: 'sp00ky-client::UpQueue::discardUnreplayable',
      },
      'Discarding an unsendable pending mutation'
    );
    try {
      await this.local.query(`DELETE $mutation_id`, {
        mutation_id: parseStoredRecordId(mutationId),
      });
    } catch (error) {
      this.logger.error(
        { error, mutationId, Category: 'sp00ky-client::UpQueue::discardUnreplayable' },
        'Failed to delete an unsendable pending mutation'
      );
    }
    this.onDropped?.({
      mutationId,
      recordId: row?.recordId,
      mutationType: row?.mutationType,
      reason,
    });
  }

  get size(): number {
    return this.queue.length;
  }

  push(event: UpEvent) {
    if (event.options?.debounced) {
      const { key, delay } = event.options.debounced;
      this.handleDebouncedMutation(event, key, delay);
      return;
    }
    this.addToQueue(event);
  }

  private addToQueue(event: UpEvent) {
    this.queue.push(event);
    this._events.addEvent({
      type: SyncQueueEventTypes.MutationEnqueued,
      payload: { queueSize: this.queue.length },
    });
  }

  private handleDebouncedMutation(event: UpEvent, key: string, delay: number) {
    const existing = this.debouncedMutations.get(key);
    let firstBeforeRecord: Record<string, unknown> | undefined;

    if (existing) {
      clearTimeout(existing.timer);
      // Preserve the beforeRecord from the first event in the debounce sequence
      firstBeforeRecord = existing.firstBeforeRecord;
    } else if (event.type === 'update') {
      firstBeforeRecord = event.beforeRecord;
    }

    const timer = setTimeout(() => {
      this.debouncedMutations.delete(key);
      // Attach the first beforeRecord to the final debounced event
      if (firstBeforeRecord && event.type === 'update') {
        event.beforeRecord = firstBeforeRecord;
      }
      this.addToQueue(event);
    }, delay);

    this.debouncedMutations.set(key, { timer, firstBeforeRecord });
  }

  /**
   * Cancel all pending debounce timers WITHOUT enqueueing their events. Used
   * on local-bucket switches: the mutation's `_00_pending_mutations` row was
   * already persisted at mutation time, so dropping the in-memory push only
   * defers it to that bucket's next `loadFromDatabase` — it must NOT be pushed
   * now, the remote session already belongs to the next user.
   */
  clearDebounceTimers(): void {
    for (const { timer } of this.debouncedMutations.values()) {
      clearTimeout(timer);
    }
    this.debouncedMutations.clear();
  }

  /**
   * @param onSettled Reports a mutation the server ACCEPTED, after its outbox
   * row is gone. Deliberately not called on the rollback path: a rejected
   * mutation must stop being rendered immediately, while an accepted one has
   * to stay visible until its membership arrives (see
   * `DataModule.noteWriteSettled`).
   */
  async next(
    fn: (event: UpEvent) => Promise<void>,
    onRollback?: RollbackCallback,
    onSettled?: (event: UpEvent) => void
  ): Promise<void> {
    const event = this.queue.shift();
    if (event) {
      try {
        await fn(event);
      } catch (error) {
        const errorType = classifySyncError(error);

        if (errorType === 'network') {
          this.logger.error(
            { error, event, Category: 'sp00ky-client::UpQueue::next' },
            'Network error processing mutation, re-queuing'
          );
          this.queue.unshift(event);
          throw error;
        }

        // Application error — rollback instead of re-queuing
        this.logger.error(
          { error, event, Category: 'sp00ky-client::UpQueue::next' },
          'Application error processing mutation, rolling back'
        );
        try {
          await this.removeEventFromDatabase(event.mutation_id);
        } catch (removeError) {
          this.logger.error(
            { error: removeError, event, Category: 'sp00ky-client::UpQueue::next' },
            'Failed to remove rolled-back mutation from database'
          );
        }
        if (onRollback) {
          try {
            await onRollback(event, error instanceof Error ? error : new Error(String(error)));
          } catch (rollbackError) {
            this.logger.error(
              { error: rollbackError, event, Category: 'sp00ky-client::UpQueue::next' },
              'Rollback handler failed'
            );
          }
        }
        this._events.addEvent({
          type: SyncQueueEventTypes.MutationDequeued,
          payload: { queueSize: this.queue.length },
        });
        return;
      }
      try {
        await this.removeEventFromDatabase(event.mutation_id);
      } catch (error) {
        this.logger.error(
          { error, event, Category: 'sp00ky-client::UpQueue::next' },
          'Failed to remove mutation from database after successful processing'
        );
      }
      // Report AFTER the outbox row is gone: that delete is exactly what drops
      // the row out of the render set, so this is the moment the grace window
      // has to start covering.
      if (onSettled) {
        try {
          onSettled(event);
        } catch (error) {
          this.logger.error(
            { error, event, Category: 'sp00ky-client::UpQueue::next' },
            'Settled-write handler failed'
          );
        }
      }
      this._events.addEvent({
        type: SyncQueueEventTypes.MutationDequeued,
        payload: { queueSize: this.queue.length },
      });
    }
  }

  async removeEventFromDatabase(mutation_id: RecordId) {
    return this.local.query(`DELETE $mutation_id`, { mutation_id });
  }

  /**
   * Shared-tabs: a follower committed an outbox row into the shared store and
   * notified the leader; enqueue exactly that row. Idempotent: a replayed
   * notify (failover) or a row already queued is a no-op, and a row that never
   * committed simply is not found (the follower's own write promise already
   * rejected in that case).
   */
  async enqueueFromDatabase(mutationId: string): Promise<void> {
    if (this.queue.some((e) => encodeUpEventId(e) === mutationId)) return;
    try {
      // ARRAY param, matching SyncEngine's `SELECT * FROM $idsToFetch`. A bare
      // `FROM $singleRecordId` looks fine against SurrealDB but the SQLite
      // engine lowers any `FROM $param` to `selectByIds` and calls `.map` on
      // the param (surql-translate.ts, SqliteCacheEngine.selectByIds), so a
      // single RecordId threw. The throw landed in the catch below, which logs
      // at `error` — invisible to an app running `logLevel: 'fatal'` — so every
      // forwarded mutation was silently dropped: the follower's optimistic
      // write stuck locally, was never pushed, and the next down-sync reverted it.
      const [records] = await this.local.query<any>(`SELECT * FROM $mutation_ids`, {
        mutation_ids: [parseRecordIdString(mutationId)],
      });
      const row = Array.isArray(records) ? records[0] : undefined;
      if (!row) return;
      const event = rowToUpEvent(row, this.logger);
      if (event) {
        this.addToQueue(event);
        return;
      }
      await this.discardUnreplayable(row, 'forwarded mutation is not replayable');
    } catch (error) {
      this.logger.error(
        { error, mutationId, Category: 'sp00ky-client::UpQueue::enqueueFromDatabase' },
        'Failed to load a forwarded mutation'
      );
      this.onDropped?.({
        mutationId,
        reason: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async loadFromDatabase() {
    try {
      // ORDER BY id: mutation ids are zero-padded-timestamp-prefixed (see
      // modules/data/mutation-id.ts), so lexicographic id order IS chronological
      // order. (The previous created_at was never a defined field on this
      // table, so the old ordering was undefined.)
      const [records] = await this.local.query<any>(
        `SELECT * FROM _00_pending_mutations ORDER BY id ASC`
      );

      const loaded: UpEvent[] = [];
      const unreplayable: any[] = [];
      for (const row of records as any[]) {
        const event = rowToUpEvent(row, this.logger);
        if (event) loaded.push(event);
        else unreplayable.push(row);
      }
      this.queue = loaded;
      // Drop them AFTER the queue is populated: one unsendable row must not
      // stop the rest of the backlog from draining, and leaving it in the store
      // would just re-poison the next boot.
      for (const row of unreplayable) {
        await this.discardUnreplayable(row, 'pending mutation is not replayable');
      }
    } catch (error) {
      this.logger.error(
        { error, Category: 'sp00ky-client::UpQueue::loadFromDatabase' },
        'Failed to load pending mutations from database'
      );
    }
  }
}

/** The stable string id of an event's outbox row. */
function encodeUpEventId(event: UpEvent): string {
  return encodeRecordId(event.mutation_id);
}

/**
 * Parse a record id READ BACK FROM THE STORE, stripping SurrealDB's `⟨⟩`
 * escaping.
 *
 * Outbox ids contain `_` and `-`, so the store hands them back in escaped
 * display form (`_00_pending_mutations:⟨1785…_0005_c960…⟩`). `parseRecordIdString`
 * keeps that verbatim, and re-encoding then escapes the brackets AGAIN, so a
 * `DELETE $mutation_id` built from a stored id targets
 * `_00_pending_mutations:⟨⟨1785…\⟩⟩` and matches nothing. Every mutation
 * replayed from the store was therefore un-deletable: its row survived a
 * SUCCESSFUL push and got re-sent on the next boot.
 */
function parseStoredRecordId(id: string): RecordId<string> {
  const [table, ...rest] = id.split(':');
  let raw = rest.join(':');
  if (raw.startsWith('⟨') && raw.endsWith('⟩')) raw = raw.slice(1, -1);
  return new RecordId(table, raw);
}

/** Materialize one `_00_pending_mutations` row into an UpEvent. */
function rowToUpEvent(r: any, logger: Logger): UpEvent | null {
  switch (r.mutationType) {
    case 'create':
      // `processUpEvent` does `Object.keys(event.data)`, so a create with no
      // payload throws before it reaches the network and can never succeed.
      // Rows written before the create branch of `surql.createMutation`
      // persisted `data` are exactly that, so refuse them here instead of
      // queueing a guaranteed failure.
      if (r.data === undefined || r.data === null) return null;
      return {
        type: 'create',
        mutation_id: parseStoredRecordId(r.id),
        record_id: parseRecordIdString(r.recordId),
        data: r.data,
        tableName: extractTablePart(r.recordId),
      };
    case 'update':
      return {
        type: 'update',
        mutation_id: parseStoredRecordId(r.id),
        record_id: parseRecordIdString(r.recordId),
        data: r.data,
        beforeRecord: r.beforeRecord,
      };
    case 'delete':
      return {
        type: 'delete',
        mutation_id: parseStoredRecordId(r.id),
        record_id: parseRecordIdString(r.recordId),
      };
    default:
      logger.warn(
        {
          mutationType: r.mutationType,
          record: r,
          Category: 'sp00ky-client::UpQueue::rowToUpEvent',
        },
        'Unknown mutation type'
      );
      return null;
  }
}
