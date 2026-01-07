import { RecordId } from 'surrealdb';
import { LocalDatabaseService } from '../database/index.js';
import { createMutationEventSystem, MutationEventSystem, MutationEventTypes } from './events.js';
import { parseRecordIdString, encodeToSpooky } from '../utils.js';
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
    private db: LocalDatabaseService,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'MutationManager' });
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

    const rid = parseRecordIdString(id);
    const tableName = rid.table.toString();
    const encodedData = encodeToSpooky(this.schema, tableName as any, data as any);

    // In SurrealDB 2.0, query returns an array of results for each statement.
    // We expect 5 statements (BEGIN, LET, LET, RETURN, COMMIT).
    // We need to find the one that contains our return object.
    const results = await this.withRetry(() =>
      this.db.query<any[]>(query, {
        id: rid,
        mid: parseRecordIdString(mutationId),
        data: encodedData,
      })
    );

    this.logger.debug({ results }, 'Create mutation response');

    // Find the result that has 'target' and 'mutation_id'
    // It might be wrapped in an array if the statement returns multiple items (SURREAL 2.0 logic?)
    // But RETURN {...} usually returns a single object or array of 1 object?
    // If query returns [r1, r2, ...], we scan them.
    const response = results.find(
      (r: any) => r && (r.target || (Array.isArray(r) && r[0]?.target))
    );
    let result = response;
    if (Array.isArray(response)) {
      result = response[0];
    }

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
          data: encodedData,
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

    const encodedData = encodeToSpooky(this.schema, table as any, data as any);

    // The return type is an array containing our custom object
    const results = await this.withRetry(() =>
      this.db.query<any[]>(query, {
        id: rid,
        data: encodedData,
      })
    );

    const response = results.find(
      (r: any) => r && (r.target || (Array.isArray(r) && r[0]?.target))
    );
    let result = response;
    if (Array.isArray(response)) {
      result = response[0];
    }

    if (!result || !result.target || (Array.isArray(result.target) && result.target.length === 0)) {
      throw new Error(`Failed to update record: ${id} not found.`);
    }

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'update',
          record_id: rid,
          data: encodedData,
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

    const results = await this.withRetry(() => this.db.query<any[]>(query, { id: rid }));

    const response = results.find(
      (r: any) => r && (r.mutation_id || (Array.isArray(r) && r[0]?.mutation_id))
    );
    let result = response;
    if (Array.isArray(response)) {
      result = response[0];
    }

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
