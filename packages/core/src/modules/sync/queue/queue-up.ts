import { RecordId } from 'surrealdb';
import { LocalDatabaseService } from '../../../services/database/index';
import {
  createSyncQueueEventSystem,
  SyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events/index';
import { parseRecordIdString, extractTablePart, classifySyncError } from '../../../utils/index';
import { Logger } from '../../../services/logger/index';
import { PushEventOptions } from '../../../events/index';

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

export class UpQueue {
  private queue: UpEvent[] = [];
  private _events: SyncQueueEventSystem;
  private logger: Logger;
  private debouncedMutations: Map<string, { timer: any; firstBeforeRecord?: Record<string, unknown> }>;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(
    private local: LocalDatabaseService,
    logger: Logger
  ) {
    this._events = createSyncQueueEventSystem();
    this.logger = logger.child({ service: 'UpQueue' });
    this.debouncedMutations = new Map();
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

  async next(fn: (event: UpEvent) => Promise<void>, onRollback?: RollbackCallback): Promise<void> {
    const event = this.queue.shift();
    if (event) {
      try {
        await fn(event);
      } catch (error) {
        const errorType = classifySyncError(error);

        if (errorType === 'network') {
          this.logger.error(
            { error, event, Category: 'spooky-client::UpQueue::next' },
            'Network error processing mutation, re-queuing'
          );
          this.queue.unshift(event);
          throw error;
        }

        // Application error — rollback instead of re-queuing
        this.logger.error(
          { error, event, Category: 'spooky-client::UpQueue::next' },
          'Application error processing mutation, rolling back'
        );
        try {
          await this.removeEventFromDatabase(event.mutation_id);
        } catch (removeError) {
          this.logger.error(
            { error: removeError, event, Category: 'spooky-client::UpQueue::next' },
            'Failed to remove rolled-back mutation from database'
          );
        }
        if (onRollback) {
          try {
            await onRollback(event, error instanceof Error ? error : new Error(String(error)));
          } catch (rollbackError) {
            this.logger.error(
              { error: rollbackError, event, Category: 'spooky-client::UpQueue::next' },
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
          { error, event, Category: 'spooky-client::UpQueue::next' },
          'Failed to remove mutation from database after successful processing'
        );
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

  async loadFromDatabase() {
    try {
      const [records] = await this.local.query<any>(
        `SELECT * FROM _spooky_pending_mutations ORDER BY created_at ASC`
      );

      this.queue = records
        .map((r: any): UpEvent | null => {
          switch (r.mutationType) {
            case 'create':
              return {
                type: 'create',
                mutation_id: parseRecordIdString(r.id),
                record_id: parseRecordIdString(r.recordId),
                data: r.data,
                tableName: extractTablePart(r.recordId),
              };
            case 'update':
              return {
                type: 'update',
                mutation_id: parseRecordIdString(r.id),
                record_id: parseRecordIdString(r.recordId),
                data: r.data,
                beforeRecord: r.beforeRecord,
              };
            case 'delete':
              return {
                type: 'delete',
                mutation_id: parseRecordIdString(r.id),
                record_id: parseRecordIdString(r.recordId),
              };
            default:
              this.logger.warn(
                {
                  mutationType: r.mutationType,
                  record: r,
                  Category: 'spooky-client::UpQueue::loadFromDatabase',
                },
                'Unknown mutation type'
              );
              return null;
          }
        })
        .filter((e: UpEvent | null): e is UpEvent => e !== null);
    } catch (error) {
      this.logger.error(
        { error, Category: 'spooky-client::UpQueue::loadFromDatabase' },
        'Failed to load pending mutations from database'
      );
    }
  }
}
