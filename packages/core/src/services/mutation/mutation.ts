import { RecordId } from 'surrealdb';
import { LocalDatabaseService } from '../database/index.js';
import { createMutationEventSystem, MutationEventSystem, MutationEventTypes } from './events.js';
import { parseRecordIdString } from '../utils.js';
import { SchemaStructure } from '@spooky/query-builder';
import { createLogger, Logger } from '../logger.js';

export class MutationManager<S extends SchemaStructure> {
  private _events: MutationEventSystem;
  private logger: Logger;

  get events(): MutationEventSystem {
    return this._events;
  }

  constructor(
    private schema: S,
    private db: LocalDatabaseService
  ) {
    this.logger = createLogger('info').child({ service: 'MutationManager' });
    this._events = createMutationEventSystem();
  }

  // Helper for retrying DB operations
  private async withRetry<T>(operation: () => Promise<T>, retries = 3, delayMs = 100): Promise<T> {
    let lastError;
    for (let i = 0; i < retries; i++) {
      try {
        return await operation();
      } catch (err: any) {
        lastError = err;
        if (
          err?.message?.includes('Can not open transaction') ||
          err?.message?.includes('transaction') ||
          err?.message?.includes('Database is busy')
        ) {
          this.logger.warn(
            {
              attempt: i + 1,
              retries,
              error: err.message,
            },
            'Retrying DB operation due to transaction error'
          );

          await new Promise((res) => setTimeout(res, delayMs * (i + 1)));
          continue;
        }
        throw err;
      }
    }
    throw lastError;
  }

  async create<T extends Record<string, unknown>>(id: string, data: T): Promise<T> {
    const mutationId = `_spooky_pending_mutations:${Date.now()}`;
    const query = `
          BEGIN TRANSACTION;
          
          LET $created = CREATE ONLY $id CONTENT $data;
          LET $mutation = CREATE ONLY $mid CONTENT {
              mutationType: 'create',
              recordId: $created.id,
              data: $data
          };

          RETURN {
              target: $created,
              mutation_id: $mutation.id
          };
          
          COMMIT TRANSACTION;
      `;

    const [response] = await this.withRetry(() =>
      this.db.query<[{ target: T; mutation_id: RecordId }]>(query, {
        id: parseRecordIdString(id),
        mid: parseRecordIdString(mutationId),
        data,
      })
    );

    this.logger.debug({ response }, 'Create mutation response');

    const result = response;

    if (!result || !result.target) {
      throw new Error('Failed to create record or mutation log.');
    }

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'create',
          mutation_id: result.mutation_id,
          record_id: result.target.id as RecordId,
          data,
        },
      ],
    });

    return result.target;
  }

  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>
  ): Promise<T> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);

    const query = `
          BEGIN TRANSACTION;

          LET $updated = UPDATE $id MERGE $data;
          LET $mutation = CREATE _spooky_pending_mutations SET 
              mutationType = 'update',
              recordId = $id,
              data = $data; 

          RETURN {
              target: $updated,
              mutation_id: $mutation.id
          };
          
          COMMIT TRANSACTION;
      `;

    // The return type is an array containing our custom object
    const [response] = await this.withRetry(() =>
      this.db.query<[{ target: T; mutation_id: RecordId }]>(query, {
        id: rid,
        data,
      })
    );

    const result = response;

    if (!result || !result.target || (Array.isArray(result.target) && result.target.length === 0)) {
      throw new Error(`Failed to update record: ${id} not found.`);
    }

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'update',
          record_id: rid,
          data,
          mutation_id: result.mutation_id,
        },
      ],
    });

    return result.target;
  }

  async delete(table: string, id: string): Promise<void> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);

    const query = `
        BEGIN TRANSACTION;
        
        DELETE $id;
        LET $mutation = CREATE _spooky_pending_mutations SET 
            mutationType = 'delete',
            recordId = $id;
        RETURN {
            mutation_id: $mutation.id
        };
        
        COMMIT TRANSACTION;
    `;

    const [response] = await this.withRetry(() =>
      this.db.query<[{ mutation_id: RecordId }]>(query, { id: rid })
    );

    const result = response;

    if (!result) {
      throw new Error('Failed to perform delete or create mutation log.');
    }

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'delete',
          record_id: rid,
          mutation_id: result.mutation_id,
        },
      ],
    });
  }
}
