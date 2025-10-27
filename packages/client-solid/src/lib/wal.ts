import { Surreal, RecordId, Table, Values } from "surrealdb";
import { ModelPayload, GenericModel } from "./models";

/**
 * Write-Ahead Log operation types
 */
export type WALOperationType = "create" | "update" | "delete";

/**
 * WAL operation record stored in internal database
 */
export interface WALOperation {
  id?: RecordId;
  operation_type: WALOperationType;
  table_name: string;
  data: any;
  record_id?: string; // For update/delete operations
  rollback_data?: any; // Snapshot of data before the operation for rollback
  created_at: Date;
  retry_count: number;
  last_error?: string;
}

/**
 * WAL Manager handles write-ahead logging for offline-first mutations
 */
export class WALManager {
  private internalDb: Surreal;
  private readonly WAL_TABLE = "wal_operations";
  private readonly MAX_RETRIES = 3;

  constructor(internalDb: Surreal) {
    this.internalDb = internalDb;
  }

  /**
   * Initialize WAL table schema in internal database
   */
  async init(): Promise<void> {
    try {
      await this.internalDb.query(`
        DEFINE TABLE IF NOT EXISTS ${this.WAL_TABLE} SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS operation_type ON TABLE ${this.WAL_TABLE} TYPE string;
        DEFINE FIELD IF NOT EXISTS table_name ON TABLE ${this.WAL_TABLE} TYPE string;
        DEFINE FIELD IF NOT EXISTS data ON TABLE ${this.WAL_TABLE} TYPE object;
        DEFINE FIELD IF NOT EXISTS record_id ON TABLE ${this.WAL_TABLE} TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS rollback_data ON TABLE ${this.WAL_TABLE} TYPE option<object>;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE ${this.WAL_TABLE} TYPE datetime VALUE time::now();
        DEFINE FIELD IF NOT EXISTS retry_count ON TABLE ${this.WAL_TABLE} TYPE int DEFAULT 0;
        DEFINE FIELD IF NOT EXISTS last_error ON TABLE ${this.WAL_TABLE} TYPE option<string>;
      `);
    } catch (error) {
      // If table already exists, that's fine
      console.log("[WAL] Table initialization completed or already exists");
    }
  }

  /**
   * Log a create operation
   */
  async logCreate(
    tableName: string,
    data: any
  ): Promise<RecordId> {
    const operation: Omit<WALOperation, "id"> = {
      operation_type: "create",
      table_name: tableName,
      data: data,
      created_at: new Date(),
      retry_count: 0,
    };

    const result = await this.internalDb
      .insert(new Table(this.WAL_TABLE), operation);

    return (result as any)[0].id!;
  }

  /**
   * Log an update operation
   */
  async logUpdate(
    tableName: string,
    recordId: RecordId,
    data: any,
    rollbackData: any
  ): Promise<RecordId> {
    const operation: Omit<WALOperation, "id"> = {
      operation_type: "update",
      table_name: tableName,
      record_id: recordId.toString(),
      data: data,
      rollback_data: rollbackData,
      created_at: new Date(),
      retry_count: 0,
    };

    const result = await this.internalDb
      .insert(new Table(this.WAL_TABLE), operation);

    return (result as any)[0].id!;
  }

  /**
   * Log a delete operation
   */
  async logDelete(
    tableName: string,
    recordId: RecordId,
    rollbackData: any
  ): Promise<RecordId> {
    const operation: Omit<WALOperation, "id"> = {
      operation_type: "delete",
      table_name: tableName,
      record_id: recordId.toString(),
      data: null, // No data for delete, but required by type
      rollback_data: rollbackData,
      created_at: new Date(),
      retry_count: 0,
    };

    const result = await this.internalDb
      .insert(new Table(this.WAL_TABLE), operation);

    return (result as any)[0].id!;
  }

  /**
   * Get all pending operations
   */
  async getPendingOperations(): Promise<WALOperation[]> {
    const [operations] = await this.internalDb
      .query(`SELECT * FROM ${this.WAL_TABLE} ORDER BY created_at ASC`)
      .collect<[WALOperation[]]>();

    return operations || [];
  }

  /**
   * Remove an operation from the WAL after successful sync
   */
  async removeOperation(operationId: RecordId): Promise<void> {
    await this.internalDb.delete(operationId);
  }

  /**
   * Update retry count and error for a failed operation
   */
  async updateOperationError(
    operationId: RecordId,
    error: string
  ): Promise<void> {
    await this.internalDb.query(
      `UPDATE $id SET retry_count += 1, last_error = $error`,
      {
        id: operationId,
        error: error,
      }
    );
  }

  /**
   * Sync all pending operations to remote database
   * Returns true if all operations succeeded, false if any failed
   */
  async syncToRemote(
    remoteDb: Surreal,
    localDb: Surreal
  ): Promise<{ success: boolean; failedOperations: WALOperation[] }> {
    const operations = await this.getPendingOperations();
    const failedOperations: WALOperation[] = [];

    console.log(`[WAL] Syncing ${operations.length} pending operations`);

    for (const operation of operations) {
      try {
        // Check if retry limit exceeded
        if (operation.retry_count >= this.MAX_RETRIES) {
          console.warn(
            `[WAL] Operation ${operation.id} exceeded max retries, skipping`
          );
          failedOperations.push(operation);
          continue;
        }

        await this.executeOperation(operation, remoteDb);

        // Successfully synced, remove from WAL
        await this.removeOperation(operation.id!);
        console.log(`[WAL] Successfully synced operation ${operation.id}`);
      } catch (error) {
        console.error(`[WAL] Failed to sync operation ${operation.id}:`, error);

        const errorMessage =
          error instanceof Error ? error.message : String(error);
        await this.updateOperationError(operation.id!, errorMessage);

        // Rollback the local change
        try {
          await this.rollbackOperation(operation, localDb);
          console.log(`[WAL] Rolled back operation ${operation.id} locally`);
        } catch (rollbackError) {
          console.error(
            `[WAL] Failed to rollback operation ${operation.id}:`,
            rollbackError
          );
        }

        failedOperations.push(operation);
      }
    }

    return {
      success: failedOperations.length === 0,
      failedOperations,
    };
  }

  /**
   * Execute a WAL operation on the remote database
   */
  private async executeOperation(
    operation: WALOperation,
    remoteDb: Surreal
  ): Promise<void> {
    switch (operation.operation_type) {
      case "create":
        await remoteDb.insert(new Table(operation.table_name), operation.data);
        break;

      case "update":
        if (!operation.record_id) {
          throw new Error("Update operation missing record_id");
        }
        const [updateTable, updateId] = operation.record_id.split(':');
        const updateRecordId = new RecordId(updateTable, updateId);
        await remoteDb.update(updateRecordId).merge(operation.data);
        break;

      case "delete":
        if (!operation.record_id) {
          throw new Error("Delete operation missing record_id");
        }
        const [deleteTable, deleteId] = operation.record_id.split(':');
        const deleteRecordId = new RecordId(deleteTable, deleteId);
        await remoteDb.delete(deleteRecordId);
        break;

      default:
        throw new Error(`Unknown operation type: ${operation.operation_type}`);
    }
  }

  /**
   * Rollback a failed operation in the local database
   */
  private async rollbackOperation(
    operation: WALOperation,
    localDb: Surreal
  ): Promise<void> {
    switch (operation.operation_type) {
      case "create":
        // Delete the locally created record
        // We need to find the record by matching the data
        // This is a best-effort approach
        await localDb.query(
          `DELETE FROM ${operation.table_name} WHERE id = $id`,
          { id: operation.data.id }
        );
        break;

      case "update":
        // Restore the original data
        if (operation.rollback_data && operation.record_id) {
          const [table, id] = operation.record_id.split(':');
          const recordId = new RecordId(table, id);
          await localDb.update(recordId).merge(operation.rollback_data);
        }
        break;

      case "delete":
        // Restore the deleted record
        if (operation.rollback_data) {
          await localDb.insert(
            new Table(operation.table_name),
            operation.rollback_data
          );
        }
        break;
    }
  }

  /**
   * Clear all operations from the WAL (use with caution)
   */
  async clearAll(): Promise<void> {
    await this.internalDb.delete(new Table(this.WAL_TABLE));
  }
}
