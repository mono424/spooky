import { RecordId } from 'surrealdb';
import { LocalDatabaseService } from '../../services/database/index.js';
import {
  createMutationEventSystem,
  MutationEventSystem,
  MutationEventTypes,
} from './events/mutation.js';
import { parseRecordIdString, encodeToSpooky } from '../../utils/index.js';
import { SchemaStructure } from '@spooky/query-builder';
import { Logger } from '../../services/logger/index.js';
import { StreamProcessorService } from '../../services/stream-processor/index.js';

export class MutationManager<S extends SchemaStructure> {
  private _events: MutationEventSystem;
  private logger: Logger;

  get events(): MutationEventSystem {
    return this._events;
  }

  constructor(
    private schema: S,
    private db: LocalDatabaseService,
    private streamProcessor: StreamProcessorService,
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

  // ==================== CREATE ====================

  async create<T extends Record<string, unknown>>(
    id: string,
    data: T,
    options?: { localOnly?: boolean }
  ): Promise<T> {
    const isLocalOnly = options?.localOnly ?? false;
    return isLocalOnly ? this.createLocalOnly<T>(id, data) : this.createLocalAndRemote<T>(id, data);
  }

  private async createLocalOnly<T extends Record<string, unknown>>(
    id: string,
    data: T
  ): Promise<T> {
    const query = `
      LET $created = CREATE ONLY $id CONTENT $data;
      RETURN {
          target: $created
      };
    `;

    const rid = parseRecordIdString(id);
    const tableName = rid.table.toString();
    const encodedData = encodeToSpooky(this.schema, tableName as any, data as any);

    const [_, result] = await this.withRetry(() =>
      this.db.query<[undefined, { target: T[] }]>(query, {
        id: rid,
        data: encodedData,
      })
    );

    this.logger.debug({ result, isLocalOnly: true }, 'Create mutation response');

    const target = result?.target?.[0];
    if (!result || !target) {
      throw new Error('Failed to create record.');
    }

    // Optimistic update: ingest record into DBSP
    this.streamProcessor.ingest(tableName, 'CREATE', id, target);

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'create',
          mutation_id: new RecordId('_spooky_pending_mutations', 'local_only'),
          record_id: parseRecordIdString(target.id as string),
          data: encodedData,
          record: target,
          localOnly: true,
        },
      ],
    });

    return target;
  }

  private async createLocalAndRemote<T extends Record<string, unknown>>(
    id: string,
    data: T
  ): Promise<T> {
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

    const [result] = await this.withRetry(() =>
      this.db.query<[{ target: T; mutation_id: string }]>(query, {
        id: rid,
        data: encodedData,
        mid: parseRecordIdString(mutationId),
      })
    );

    this.logger.debug({ result, isLocalOnly: false }, 'Create mutation response');

    const target = result?.target;
    const createdMutationId = result?.mutation_id;

    if (!result || !target || !createdMutationId) {
      throw new Error('Failed to create record or mutation log.');
    }

    // Optimistic update: ingest record into DBSP
    this.streamProcessor.ingest(tableName, 'CREATE', id, target);

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'create',
          mutation_id: parseRecordIdString(createdMutationId),
          record_id: parseRecordIdString(target.id as string),
          data: encodedData,
          record: target,
          localOnly: false,
        },
      ],
    });

    return target;
  }

  // ==================== UPDATE ====================

  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>,
    options?: { localOnly?: boolean }
  ): Promise<T> {
    const isLocalOnly = options?.localOnly ?? false;
    return isLocalOnly
      ? this.updateLocalOnly<T>(table, id, data)
      : this.updateLocalAndRemote<T>(table, id, data);
  }

  private async updateLocalOnly<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>
  ): Promise<T> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);
    const query = `
      LET $updated = UPDATE ONLY $id MERGE $data;
      RETURN {
          target: $updated
      };
    `;

    const encodedData = encodeToSpooky(this.schema, table as any, data as any);

    const [result] = await this.withRetry(() =>
      this.db.query<[{ target: T[] }]>(query, {
        id: rid,
        data: encodedData,
      })
    );

    const target = result?.target?.[0];
    if (!result || !target) {
      throw new Error(`Failed to update record: ${id} not found.`);
    }

    // Optimistic update: ingest record into DBSP
    this.streamProcessor.ingest(table, 'update', id, target);

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'update',
          record_id: rid,
          data: encodedData,
          record: target,
          mutation_id: new RecordId('_spooky_pending_mutations', 'local_only'),
          localOnly: true,
        },
      ],
    });

    return target;
  }

  private async updateLocalAndRemote<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>
  ): Promise<T> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);
    const query = `
      BEGIN TRANSACTION;

      LET $updated = UPDATE ONLY $id MERGE $data;
      LET $mutation = CREATE ONLY _spooky_pending_mutations SET 
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

    const [result] = await this.withRetry(() =>
      this.db.query<[{ target: T; mutation_id: string }]>(query, {
        id: rid,
        data: encodedData,
      })
    );

    const target = result?.target;
    const createdMutationId = result?.mutation_id;

    if (!result || !target || !createdMutationId) {
      throw new Error(`Failed to update record: ${id} or create mutation log.`);
    }

    // Optimistic update: ingest record into DBSP
    this.streamProcessor.ingest(table, 'update', id, target);

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'update',
          record_id: rid,
          data: encodedData,
          record: target,
          mutation_id: parseRecordIdString(createdMutationId),
          localOnly: false,
        },
      ],
    });

    return target;
  }

  // ==================== DELETE ====================

  async delete(table: string, id: string, options?: { localOnly?: boolean }): Promise<void> {
    const isLocalOnly = options?.localOnly ?? false;
    return isLocalOnly ? this.deleteLocalOnly(table, id) : this.deleteLocalAndRemote(table, id);
  }

  private async deleteLocalOnly(table: string, id: string): Promise<void> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);
    const query = `
      DELETE $id;
      RETURN { success: true }; 
    `;

    await this.withRetry(() => this.db.query<any[]>(query, { id: rid }));

    // Optimistic update: ingest delete into DBSP
    this.streamProcessor.ingest(table, 'DELETE', id, {});

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'delete',
          record_id: rid,
          mutation_id: new RecordId('_spooky_pending_mutations', 'local_only'),
          localOnly: true,
        },
      ],
    });
  }

  private async deleteLocalAndRemote(table: string, id: string): Promise<void> {
    const rid = id.includes(':') ? parseRecordIdString(id) : new RecordId(table, id);
    const query = `
      BEGIN TRANSACTION;
      
      DELETE $id;
      LET $mutation = CREATE ONLY _spooky_pending_mutations SET 
          mutationType = 'delete',
          recordId = $id;
      RETURN {
          mutation_id: $mutation.id
      };
      
      COMMIT TRANSACTION;
    `;

    const [result] = await this.withRetry(() =>
      this.db.query<{ mutation_id: string }[]>(query, { id: rid })
    );

    if (!result) {
      throw new Error('Failed to perform delete or create mutation log.');
    }

    const resultMutationId = parseRecordIdString(result.mutation_id);

    // Optimistic update: ingest delete into DBSP
    this.streamProcessor.ingest(table, 'DELETE', id, {});

    this._events.addEvent({
      type: MutationEventTypes.MutationCreated,
      payload: [
        {
          type: 'delete',
          record_id: rid,
          mutation_id: resultMutationId,
          localOnly: false,
        },
      ],
    });
  }
}
