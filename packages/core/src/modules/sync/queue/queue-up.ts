import { RecordId } from 'surrealdb';
import { MutationEventSystem, MutationEventTypes } from '../../data/events/mutation.js';
import { LocalDatabaseService } from '../../../services/database/index.js';
import {
  createSyncQueueEventSystem,
  SyncQueueEventSystem,
  SyncQueueEventTypes,
} from '../events/index.js';
import { parseRecordIdString } from '../../../utils/index.js';

export type CreateEvent = {
  type: 'create';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
  localOnly?: boolean;
};

export type UpdateEvent = {
  type: 'update';
  mutation_id: RecordId;
  record_id: RecordId;
  data: Record<string, unknown>;
  record?: Record<string, unknown>;
  localOnly?: boolean;
};

export type DeleteEvent = {
  type: 'delete';
  mutation_id: RecordId;
  record_id: RecordId;
  localOnly?: boolean;
};

export type UpEvent = CreateEvent | UpdateEvent | DeleteEvent;

export class UpQueue {
  private queue: UpEvent[] = [];
  private _events: SyncQueueEventSystem;

  get events(): SyncQueueEventSystem {
    return this._events;
  }

  constructor(private local: LocalDatabaseService) {
    this._events = createSyncQueueEventSystem();
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
        console.error('Failed to process mutation', event, error);
        this.queue.unshift(event);
        throw error;
      }
      try {
        await this.removeEventFromDatabase(event.mutation_id);
      } catch (error) {
        // TODO: handle this, we still have this mutation in the database, eventough it
        // was processed successfully
        console.error('Failed to remove mutation from database', event, error);
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
              console.warn(`Unknown mutation type: ${r.mutationType}`, r);
              return null;
          }
        })
        .filter((e: UpEvent | null): e is UpEvent => e !== null);
    } catch (error) {
      console.error('Failed to load pending mutations from database:', error);
      // TODO: clarify if we want to throw or not
      // Don't crash, just start with empty queue? Or throw?
      // For now, logging is safer.
    }
  }
}
