import { RecordId } from 'surrealdb';
import { LocalDatabaseService } from '../../../services/database/index.js';
import {
  createSyncQueueEventSystem,
  SyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events/index.js';
import { parseRecordIdString } from '../../../utils/index.js';
import { Logger } from '../../../services/logger/index.js';

export type CreateEvent = {
  type: 'create';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
};

export type UpdateEvent = {
  type: 'update';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
};

export type DeleteEvent = {
  type: 'delete';
  mutation_id: RecordId;
  record_id: RecordId;
};

export type UpEvent = CreateEvent | UpdateEvent | DeleteEvent;

export class UpQueue {
  private queue: UpEvent[] = [];
  private _events: SyncQueueEventSystem;
  private logger: Logger;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(
    private local: LocalDatabaseService,
    logger: Logger
  ) {
    this._events = createSyncQueueEventSystem();
    this.logger = logger.child({ service: 'UpQueue' });
  }

  get size(): number {
    return this.queue.length;
  }

  push(event: UpEvent) {
    this.queue.push(event);
    this._events.addEvent({
      type: SyncQueueEventTypes.MutationEnqueued,
      payload: { queueSize: this.queue.length },
    });
  }

  async next(fn: (event: UpEvent) => Promise<void>): Promise<void> {
    const event = this.queue.shift();
    if (event) {
      try {
        await fn(event);
      } catch (error) {
        this.logger.error(
          { error, event, Category: 'spooky-client::UpQueue::next' },
          'Failed to process mutation'
        );
        this.queue.unshift(event);
        throw error;
      }
      try {
        await this.removeEventFromDatabase(event.mutation_id);
      } catch (error) {
        this.logger.error(
          { error, event, Category: 'spooky-client::UpQueue::next' },
          'Failed to remove mutation from database after successful processing'
        );
      }
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
              };
            case 'update':
              return {
                type: 'update',
                mutation_id: parseRecordIdString(r.id),
                record_id: parseRecordIdString(r.recordId),
                data: r.data,
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
